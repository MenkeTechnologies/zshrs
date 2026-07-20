//! Port of `_mailboxes` from `Completion/Unix/Type/_mailboxes`.
//!
//! Full upstream body (198 lines, abridged) — three shell functions:
//! ```text
//! sh:  3  _mailboxes()      main: pick tags by MUA (curcontext), _tags loop
//! sh:                       → _requested mailboxes _mua_mailboxes / _files.
//! sh: 63  _mailbox_cache()  build the -U caches (_mbox/_maildir/_mh/_pine/
//! sh:                       _mutt/_mailbox) by globbing $maildirectory.
//! sh:114  _mua_mailboxes()  per-MUA name list (compset prefixes +/@/%/=),
//! sh:                       then _multi_parts / compadd.
//! sh:198  _mailboxes "$@"
//! ```
//!
//! Approximations (marked `// sh:N approx`): the muttrc `mailboxes` eval
//! (sh:76) is not parsed; `elm`/`tkrat` custom formats are folded into the
//! default arms as the source itself notes; `~`-expansion uses `$HOME`.

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

fn get_cache(name: &str) -> Vec<String> {
    getaparam(name).unwrap_or_default()
}

/// sh:63-112 — populate the `-U` caches by globbing `$maildirectory`.
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
    let maildir = tilde(&maildirectory);

    // sh:76 approx — the muttrc `mailboxes …` eval is not parsed.
    let mut mutt_cache: Vec<String> = Vec::new();
    let _ = &mut mutt_cache;

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

    setaparam("_mbox_cache", mbox_cache);
    setaparam("_maildir_cache", maildir_cache);
    setaparam("_mh_cache", mh_cache);
    setaparam("_pine_cache", pine_cache);
    setaparam("_mutt_cache", mutt_cache);
    setaparam("_mailbox_cache", mailbox_cache);
}

/// sh:114-196 — MUA-specific mailbox-name completion.
fn mua_mailboxes(args: &[String]) -> i32 {
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

    if is("mutt") {
        // sh:157-172
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
    } else if is("mh") {
        // sh:129-141
        if compset(&["-P", "+"]) == 0 {
            mbox_names = strip_dir_prefix(&mh, &maildirectory);
        } else {
            mbox_names = mh
                .iter()
                .map(|e| {
                    format!(
                        "+{}",
                        e.strip_prefix(&format!("{}/", maildirectory)).unwrap_or(e)
                    )
                })
                .collect();
            mbox_names.extend(mh.clone());
        }
    } else if is("pine") {
        // sh:174-183
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
    } else {
        // sh:191-194 — default: everything.
        mbox_names.extend(mailbox);
        mbox_names.extend(mbox);
        mbox_names.extend(mh);
        mbox_names.extend(mutt);
        mbox_names.extend(pine);
    }

    let mut ret = 1;
    // sh:189 — (( $#mbox_names )) && _multi_parts "$@" / mbox_names
    if !mbox_names.is_empty() {
        setaparam("mbox_names", mbox_names);
        let mut mp: Vec<String> = args.to_vec();
        mp.push("/".to_string());
        mp.push("mbox_names".to_string());
        if _multi_parts(&mp) == 0 {
            ret = 0;
        }
    }
    // sh:190 — (( $#mbox_short )) && compadd "$@" -a mbox_short
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
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let mut ret = 1;

    // sh:11-13 — build the caches once.
    if getaparam("_mailbox_cache").is_none() {
        mailbox_cache();
    }

    let prefix = getsparam("PREFIX").unwrap_or_default();
    // sh:15-47 — choose tags based on MUA + PREFIX.
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
        if _requested(&[
            "mailboxes".to_string(),
            "expl".to_string(),
            "mailbox specification".to_string(),
            "_mua_mailboxes".to_string(),
        ]) == 0
            && mua_mailboxes(args) == 0
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
            if _files(&expl) == 0 {
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
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setsparam("curcontext", ":completion::mail:::");
        crate::ported::params::setsparam("PREFIX", "");
        setaparam("_mailbox_cache", Vec::new());
        assert_eq!(_mailboxes(&[]), 1);
    }
}
