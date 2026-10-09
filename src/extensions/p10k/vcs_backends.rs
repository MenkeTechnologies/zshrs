//! The remaining `vcs_info` backends the p10k `vcs` segment can select
//! through `POWERLEVEL9K_VCS_BACKENDS`: bzr, cdv, cvs, darcs, fossil, mtn,
//! p4, svk and tla (hg and svn are in `vcs_other.rs`; git is the gitstatus
//! port). Each is a port of `Functions/VCS_Info/Backends/VCS_INFO_detect_X`
//! and `VCS_INFO_get_data_X`: the command line, the parsing and the
//! arguments handed to `VCS_INFO_formats` are the originals'; the rendering
//! (formats, hooks, icons) is the shared pipeline in `vcs_other.rs`.
//!
//! The parsers are free functions over captured command output so they can
//! be tested without the tools installed.
//!
//! Hash-ordered output: where a backend prints an associative array's keys
//! (`${(k)counts}`, `${(Mk)fsinfo:#...}`) the order is zsh's bucket order
//! ([`zsh_hash_order`]), not insertion order.

use crate::extensions::p10k::vcs_hooks::{zsh_hash_order, zsh_tail, Assoc};
use crate::extensions::p10k::vcs_other::{bydir_detect, tool_bin, Run};
use crate::ported::params::getsparam;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ---------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------

/// `IFS=: read a b` over every line: `a` up to the first colon, `b` the
/// rest. Lines without a colon are all `a`. Empty keys are skipped (zsh
/// rejects an empty subscript).
fn read_colon_pairs(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter(|l| !l.is_empty())
        .map(|l| match l.split_once(':') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => (l.to_string(), String::new()),
        })
        .collect()
}

/// The `${(f)...}` words of `$(cmd)` with trailing newlines removed.
fn chomp(s: &str) -> &str {
    s.trim_end_matches('\n')
}

// ---------------------------------------------------------------------
// bzr
// ---------------------------------------------------------------------

/// `bzr info`: the first line (the tree kind) and the `key: value` lines
/// matching `^[ a-zA-Z0-9]\+: `, keys with spaces turned to `_`
/// (`read` has trimmed each line).
fn parse_bzr_info(out: &str) -> (String, Assoc) {
    let mut lines = out.lines();
    let dirtype = lines.next().unwrap_or("").trim().to_string();
    let mut info = Assoc::default();
    for raw in lines {
        let Some(colon) = raw.find(':') else { continue };
        let prefix_ok = colon > 0
            && raw[..colon].chars().all(|c| c == ' ' || c.is_ascii_alphanumeric())
            && raw[colon + 1..].starts_with(' ');
        if !prefix_ok {
            continue;
        }
        let line = raw.trim();
        let key = line.split(": ").next().unwrap_or("").replace(' ', "_");
        let value = line.split_once(": ").map(|(_, v)| v).unwrap_or("");
        info.set(&key, value);
    }
    (dirtype, info)
}

#[derive(Debug, PartialEq)]
enum BzrKind {
    Checkout,
    Lightweight,
    Standalone,
}

/// The tree kind and its root (`bzrbase` before `:P`).
fn bzr_kind_and_root(dirtype: &str, info: &Assoc) -> (BzrKind, String) {
    if dirtype.starts_with("Checkout") {
        (BzrKind::Checkout, info.get("checkout_root").to_string())
    } else if dirtype.starts_with("Repository checkout") {
        (BzrKind::Checkout, info.get("repository_checkout_root").to_string())
    } else if dirtype.starts_with("Lightweight checkout") {
        (BzrKind::Lightweight, info.get("light_checkout_root").to_string())
    } else {
        (BzrKind::Standalone, info.get("branch_root").to_string())
    }
}

/// `${(s.:.)$(bzr version-info --custom --template="{revno}:{branch_nick}:{clean}")}`
/// — empty fields vanish.
fn parse_bzr_version_info(out: &str) -> Vec<String> {
    out.split(':').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// `VCS_INFO_bzr_get_changes`: `bzr stat -SV` counted per status flag,
/// printed `flag:count ` in hash order.
fn parse_bzr_stat(out: &str) -> String {
    let mut counts: Vec<(String, u32)> = Vec::new();
    for line in out.lines() {
        let Some(flag) = line.split_whitespace().next() else { continue };
        match counts.iter_mut().find(|(f, _)| f == flag) {
            Some(slot) => slot.1 += 1,
            None => counts.push((flag.to_string(), 1)),
        }
    }
    let keys: Vec<String> = counts.iter().map(|(f, _)| f.clone()).collect();
    zsh_hash_order(&keys)
        .iter()
        .map(|k| format!("{k}:{} ", counts.iter().find(|(f, _)| f == k).map_or(0, |c| c.1)))
        .collect()
}

/// `${bzrinfo[i]}` — zsh arrays are 1-based and read empty past the end.
fn nth(v: &[String], i: usize) -> &str {
    v.get(i - 1).map(String::as_str).unwrap_or("")
}

impl Run<'_> {
    /// `:P` — the physical path of `p` (relative to the cwd).
    fn physical(&self, p: &str) -> String {
        let joined = if p.is_empty() { self.cwd.clone() } else { self.cwd.join(p) };
        joined.canonicalize().unwrap_or(joined).to_string_lossy().into_owned()
    }

    fn require_cmd(&self) -> bool {
        tool_bin(self.st.vcs_comm.get("cmd")).is_some()
    }

    fn set_basedir(&mut self, base: PathBuf) {
        self.st.vcs_comm.set("basedir", base.to_string_lossy().into_owned());
    }

    fn basedir(&self) -> String {
        self.st.vcs_comm.get("basedir").to_string()
    }

    /// `VCS_INFO_bzr_get_changes`.
    fn bzr_changes(&self) -> String {
        self.tool(&["stat", "-SV"], &self.cwd, &[])
            .map(|o| parse_bzr_stat(&o.stdout))
            .unwrap_or_default()
    }

    pub(crate) fn detect_bzr(&mut self) -> bool {
        // detect_bzr checks the literal command `bzr`, not `vcs_comm[cmd]`.
        if tool_bin("bzr").is_none() {
            return false;
        }
        match bydir_detect(&self.cwd, ".bzr", &["branch/format"]) {
            Some(b) => {
                self.set_basedir(b);
                true
            }
            None => false,
        }
    }

    pub(crate) fn get_data_bzr(&mut self) -> bool {
        let bzrinfo: Vec<String>;
        let mut changes = String::new();
        let bzrbase: String;
        if self.style_t("use-simple") {
            bzrbase = self.basedir();
            let last = std::fs::read_to_string(format!("{bzrbase}/.bzr/branch/last-revision")).ok();
            let revno = last
                .map(|s| s.trim_end_matches('\n').split(' ').next().unwrap_or("").to_string())
                .unwrap_or_default();
            bzrinfo = vec![revno, zsh_tail(&bzrbase).to_string()];
        } else {
            let info_out = self.tool(&["info"], &self.cwd, &[]).map(|o| o.stdout).unwrap_or_default();
            let (dirtype, info) = parse_bzr_info(&info_out);
            let (kind, root) = bzr_kind_and_root(&dirtype, &info);
            bzrbase = self.physical(&root);
            let cob = info.get("checkout_of_branch");
            let restricted = !(!cob.is_empty() && self.style_t("use-server"))
                && !cob.starts_with("file://")
                && cob.contains("://");
            if restricted {
                // VCS_INFO_bzr_get_info_restricted
                let revno = self.tool(&["revno"], &self.cwd, &[]).map(|o| o.stdout).unwrap_or_default();
                let mut words: Vec<String> = revno.split_whitespace().map(str::to_string).collect();
                words.push(zsh_tail(&bzrbase).to_string());
                bzrinfo = words;
                if self.style_t("check-for-changes") && kind != BzrKind::Lightweight {
                    changes = self.bzr_changes();
                }
            } else {
                // VCS_INFO_bzr_get_info
                let tpl = "{revno}:{branch_nick}:{clean}";
                let out = self
                    .tool(&["version-info", "--custom", &format!("--template={tpl}")], &self.cwd, &[])
                    .map(|o| o.stdout)
                    .unwrap_or_default();
                bzrinfo = parse_bzr_version_info(chomp(&out));
                if self.style_t("check-for-changes") {
                    changes = self.bzr_changes();
                }
            }
        }
        self.st.vars.rrn = zsh_tail(&bzrbase).to_string();
        let branch = self
            .set_branch_format(nth(&bzrinfo, 2), nth(&bzrinfo, 1))
            .unwrap_or_default();
        self.formats("", &branch, &bzrbase, "", &changes, nth(&bzrinfo, 1), &changes);
        true
    }
}

// ---------------------------------------------------------------------
// cdv, darcs, cvs, tla, mtn — name-only backends
// ---------------------------------------------------------------------

/// mtn: `Current branch: x` line of `mtn status`, everything through the
/// last `: ` removed, matches joined by a space.
fn parse_mtn_branch(out: &str) -> String {
    out.lines()
        .filter(|l| l.starts_with("Current branch:"))
        .map(|l| l.rsplit_once(": ").map_or(l, |(_, b)| b))
        .collect::<Vec<_>>()
        .join(" ")
}

/// tla: `${${"$(tla tree-id)"}/*\//}` — through the last `/` removed.
fn tla_branch(tree_id: &str) -> &str {
    let t = chomp(tree_id);
    t.rsplit_once('/').map_or(t, |(_, b)| b)
}

/// cvs: `${${cvsbranch}##${rrn}/}`, falling back to the directory name.
fn cvs_branch(repository: &str, rrn: &str) -> String {
    let repo = chomp(repository);
    let stripped = repo.strip_prefix(&format!("{rrn}/")).unwrap_or(repo);
    if stripped.is_empty() { rrn.to_string() } else { stripped.to_string() }
}

impl Run<'_> {
    /// The shared shape of cdv/darcs/mtn/tla: a base directory, branch
    /// text, nothing else.
    fn name_only(&mut self, base: &str, branch: &str) {
        self.st.vars.rrn = zsh_tail(base).to_string();
        self.formats("", branch, base, "", "", "", "");
    }

    pub(crate) fn detect_cdv(&mut self) -> bool {
        if !self.require_cmd() {
            return false;
        }
        match bydir_detect(&self.cwd, ".cdv", &["format"]) {
            Some(b) => {
                self.set_basedir(b);
                true
            }
            None => false,
        }
    }

    pub(crate) fn get_data_cdv(&mut self) -> bool {
        let base = self.basedir();
        self.name_only(&base, zsh_tail(&base).to_string().as_str());
        true
    }

    pub(crate) fn detect_darcs(&mut self) -> bool {
        if !self.require_cmd() {
            return false;
        }
        match bydir_detect(&self.cwd, "_darcs", &["format"]) {
            Some(b) => {
                self.set_basedir(b);
                true
            }
            None => false,
        }
    }

    pub(crate) fn get_data_darcs(&mut self) -> bool {
        let base = self.basedir();
        self.name_only(&base, zsh_tail(&base).to_string().as_str());
        true
    }

    pub(crate) fn detect_mtn(&mut self) -> bool {
        if !self.require_cmd() {
            return false;
        }
        match bydir_detect(&self.cwd, "_MTN", &["revision"]) {
            Some(b) => {
                self.set_basedir(b);
                true
            }
            None => false,
        }
    }

    pub(crate) fn get_data_mtn(&mut self) -> bool {
        let base = self.basedir();
        let branch = self
            .tool(&["status"], &self.cwd, &[])
            .map(|o| parse_mtn_branch(&o.stdout))
            .unwrap_or_default();
        self.name_only(&base, &branch);
        true
    }

    pub(crate) fn detect_tla(&mut self) -> bool {
        if !self.require_cmd() {
            return false;
        }
        match self.tool(&["tree-root"], &self.cwd, &[]) {
            Some(o) if o.code == Some(0) => {
                self.st.vcs_comm.set("basedir", chomp(&o.stdout));
                true
            }
            _ => false,
        }
    }

    pub(crate) fn get_data_tla(&mut self) -> bool {
        let base = self.physical(&self.basedir());
        let tree_id = self.tool(&["tree-id"], &self.cwd, &[]).map(|o| o.stdout).unwrap_or_default();
        self.name_only(&base, tla_branch(&tree_id));
        true
    }

    pub(crate) fn detect_cvs(&mut self) -> bool {
        if !self.require_cmd() {
            return false;
        }
        let cvs = self.cwd.join("CVS");
        if !cvs.is_dir() || std::fs::File::open(cvs.join("Repository")).is_err() {
            return false;
        }
        let mut base = self.cwd.clone();
        while let Some(parent) = base.parent().map(Path::to_path_buf) {
            if !parent.join("CVS").is_dir() {
                break;
            }
            base = parent;
            if base == Path::new("/") {
                break;
            }
        }
        self.set_basedir(base);
        true
    }

    pub(crate) fn get_data_cvs(&mut self) -> bool {
        let base = self.basedir();
        let repo = std::fs::read_to_string(self.cwd.join("CVS/Repository")).unwrap_or_default();
        let rrn = zsh_tail(&base).to_string();
        self.st.vars.rrn = rrn.clone();
        self.formats("", &cvs_branch(&repo, &rrn), &base, "", "", "", "");
        true
    }
}

// ---------------------------------------------------------------------
// fossil
// ---------------------------------------------------------------------

/// What get_data_fossil derives from `fossil status`.
#[derive(Debug, PartialEq)]
struct FossilStatus {
    hash: String,
    branch: String,
    changed: String,
    action: &'static str,
    local_root: String,
    repository: String,
}

fn parse_fossil_status(out: &str) -> FossilStatus {
    let mut info = Assoc::default();
    let mut keys: Vec<String> = Vec::new();
    for (a, b) in read_colon_pairs(out) {
        let key = a.replace('-', "_");
        if !info.has(&key) {
            keys.push(key.clone());
        }
        info.set(&key, b.trim_start_matches(' '));
    }
    let ordered = zsh_hash_order(&keys);
    let changed = ordered
        .iter()
        .filter(|k| ["ADDED", "EDITED", "DELETED", "UPDATED"].iter().any(|p| k.starts_with(p)))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let merging = keys.iter().any(|k| k.contains("_BY_MERGE"));
    FossilStatus {
        hash: info.get("checkout").split(' ').next().unwrap_or("").to_string(),
        branch: info.get("tags").split(", ").next().unwrap_or("").to_string(),
        changed,
        action: if merging { "merging" } else { "" },
        local_root: info.get("local_root").to_string(),
        repository: info.get("repository").to_string(),
    }
}

impl Run<'_> {
    pub(crate) fn detect_fossil(&mut self) -> bool {
        if !self.require_cmd() {
            return false;
        }
        match bydir_detect(&self.cwd, ".", &["_FOSSIL_", ".fslckout"]) {
            Some(b) => {
                self.set_basedir(b);
                true
            }
            None => false,
        }
    }

    pub(crate) fn get_data_fossil(&mut self) -> bool {
        let out = self.tool(&["status"], &self.cwd, &[]).map(|o| o.stdout).unwrap_or_default();
        let s = parse_fossil_status(&out);
        self.st.vars.rrn = zsh_tail(&s.local_root).to_string();
        self.formats(s.action, &s.branch, &s.local_root, "", &s.changed, &s.hash, &s.repository);
        true
    }
}

// ---------------------------------------------------------------------
// p4
// ---------------------------------------------------------------------

/// `VCS_INFO_detect_p4`'s record of servers that refused a connection
/// (`vcs_info_p4_dead_servers`), kept for the life of the shell.
static P4_DEAD_SERVERS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `VCS_INFO_p4_get_server`: `P4PORT` from `p4 set` with the defaults for
/// an empty host or port.
fn p4_server_port(settings: &str) -> String {
    let port = settings
        .lines()
        .find_map(|l| l.strip_prefix("P4PORT="))
        .map(|v| v.split(' ').next().unwrap_or("").to_string())
        .unwrap_or_default();
    if port.is_empty() || port == ":" {
        "perforce:1666".to_string()
    } else if port.starts_with(':') {
        format!("perforce{port}")
    } else if port.ends_with(':') {
        format!("{port}1666")
    } else if port.bytes().all(|b| b.is_ascii_digit()) {
        format!("perforce:{port}")
    } else {
        port
    }
}

/// get_data_p4: `${${$(p4 changes -m 1 ...#have)##Change }%% *}`.
fn parse_p4_change(out: &str) -> String {
    let t = chomp(out);
    let t = t.strip_prefix("Change ").unwrap_or(t);
    t.split(' ').next().unwrap_or("").to_string()
}

impl Run<'_> {
    fn p4_server(&self) -> String {
        let settings = self.tool(&["set"], &self.cwd, &[]).map(|o| o.stdout).unwrap_or_default();
        p4_server_port(&settings)
    }

    pub(crate) fn detect_p4(&mut self) -> bool {
        if self.style_t("use-server") {
            let mut serverport = String::new();
            let any_dead = P4_DEAD_SERVERS.lock().map(|d| !d.is_empty()).unwrap_or(false);
            if any_dead {
                serverport = self.p4_server();
                if P4_DEAD_SERVERS.lock().map(|d| d.contains(&serverport)).unwrap_or(false) {
                    return false;
                }
            }
            let Some(o) = self.tool(&["where"], &self.cwd, &[]) else {
                return false;
            };
            if o.code == Some(0) {
                return true;
            }
            if format!("{}{}", o.stdout, o.stderr).contains("Connect to server failed") {
                if serverport.is_empty() {
                    serverport = self.p4_server();
                }
                if let Ok(mut d) = P4_DEAD_SERVERS.lock() {
                    d.push(serverport);
                }
            }
            return false;
        }
        let config = getsparam("P4CONFIG").unwrap_or_default();
        if config.is_empty() || !self.require_cmd() {
            return false;
        }
        match bydir_detect(&self.cwd, ".", &[config.as_str()]) {
            Some(b) => {
                self.set_basedir(b);
                true
            }
            None => false,
        }
    }

    pub(crate) fn get_data_p4(&mut self) -> bool {
        let info_out = self.tool(&["info"], &self.cwd, &[]).map(|o| o.stdout).unwrap_or_default();
        let mut p4info = Assoc::default();
        for (a, b) in read_colon_pairs(&info_out) {
            p4info.set(&a.replace(' ', "_"), b.trim_start_matches(' '));
        }
        let base = self.basedir();
        let changes = self
            .tool(&["changes", "-m", "1", "...#have"], &self.cwd, &[])
            .map(|o| o.stdout)
            .unwrap_or_default();
        let change = parse_p4_change(&changes);
        self.st.vars.rrn = zsh_tail(&base).to_string();
        let branch = self.set_branch_format(p4info.get("Client_name"), &change).unwrap_or_default();
        self.formats("", &branch, &base, "", "", &change, "");
        true
    }
}

// ---------------------------------------------------------------------
// svk
// ---------------------------------------------------------------------

/// `(basedir, branch, revision)` found in `~/.svk/config` for `pwd`
/// (`VCS_INFO_detect_svk`'s read loop).
fn parse_svk_config(config: &str, pwd: &str) -> Option<(String, String, String)> {
    let (mut basedir, mut branch, mut revision) = (String::new(), String::new(), String::new());
    let mut fhash = false;
    for raw in config.lines() {
        if !basedir.is_empty() {
            let line = raw.trim_start_matches(' ');
            if line.starts_with("depotpath:") {
                branch = line.rsplit('/').next().unwrap_or("").to_string();
            }
            if line.starts_with("revision:") {
                // `${line##*[[:space:]]##}`: text after the last whitespace run
                revision = line.rsplit(char::is_whitespace).next().unwrap_or(line).to_string();
            }
            if !branch.is_empty() && !revision.is_empty() {
                break;
            }
            continue;
        }
        if fhash {
            let mut chars = raw.chars();
            let starts = raw.starts_with("  ");
            let third = raw.chars().nth(2);
            if starts && third.map_or(false, |c| !c.is_whitespace()) {
                chars.nth(2);
                if chars.as_str().contains(':') {
                    break;
                }
            }
        }
        if raw.starts_with("  hash:") {
            fhash = true;
            continue;
        }
        if !fhash {
            continue;
        }
        let stripped = raw.trim_start_matches(' ');
        let candidate = stripped.rsplit_once(':').map_or(stripped, |(h, _)| h);
        if format!("{pwd}/").starts_with(&format!("{candidate}/")) {
            basedir = candidate.to_string();
        }
    }
    (!basedir.is_empty() && !branch.is_empty() && !revision.is_empty())
        .then_some((basedir, branch, revision))
}

impl Run<'_> {
    pub(crate) fn detect_svk(&mut self) -> bool {
        if !self.require_cmd() {
            return false;
        }
        let home = getsparam("HOME").unwrap_or_default();
        let Ok(config) = std::fs::read_to_string(format!("{home}/.svk/config")) else {
            return false;
        };
        match parse_svk_config(&config, &self.pwd) {
            Some((base, branch, revision)) => {
                self.st.vcs_comm.set("basedir", base);
                self.st.vcs_comm.set("branch", branch);
                self.st.vcs_comm.set("revision", revision);
                true
            }
            None => false,
        }
    }

    pub(crate) fn get_data_svk(&mut self) -> bool {
        let base = self.basedir();
        let (branch_in, revision) = (
            self.st.vcs_comm.get("branch").to_string(),
            self.st.vcs_comm.get("revision").to_string(),
        );
        self.st.vars.rrn = zsh_tail(&base).to_string();
        let branch = self.set_branch_format(&branch_in, &revision).unwrap_or_default();
        self.formats("", &branch, &base, "", "", &revision, "");
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_pairs_split_at_the_first_colon_only() {
        let p = read_colon_pairs("checkout:  abc 2024-05-01 10:11:12 UTC\nEDITED     a.c\n\n");
        assert_eq!(
            p,
            vec![
                ("checkout".to_string(), "  abc 2024-05-01 10:11:12 UTC".to_string()),
                ("EDITED     a.c".to_string(), String::new()),
            ]
        );
    }

    const BZR_STANDALONE: &str = "Standalone tree (format: 2a)\n\
        Location:\n\
        \x20 branch root: .\n\
        \n\
        Related branches:\n\
        \x20 parent branch: bzr+ssh://host/project\n";

    #[test]
    fn bzr_info_standalone() {
        let (dirtype, info) = parse_bzr_info(BZR_STANDALONE);
        assert_eq!(dirtype, "Standalone tree (format: 2a)");
        assert_eq!(info.get("branch_root"), ".");
        assert_eq!(info.get("parent_branch"), "bzr+ssh://host/project");
        assert!(!info.has("Location"));
        assert_eq!(bzr_kind_and_root(&dirtype, &info), (BzrKind::Standalone, ".".to_string()));
    }

    #[test]
    fn bzr_info_checkout_kinds() {
        let co = "Checkout (format: unnamed)\nLocation:\n       checkout root: /w/co\n  checkout of branch: bzr+ssh://host/trunk\n";
        let (t, info) = parse_bzr_info(co);
        assert_eq!(bzr_kind_and_root(&t, &info), (BzrKind::Checkout, "/w/co".to_string()));
        assert_eq!(info.get("checkout_of_branch"), "bzr+ssh://host/trunk");

        let rco = "Repository checkout (format: 2a)\nLocation:\n  repository checkout root: /w/rco\n";
        let (t, info) = parse_bzr_info(rco);
        assert_eq!(bzr_kind_and_root(&t, &info), (BzrKind::Checkout, "/w/rco".to_string()));

        let lw = "Lightweight checkout (format: 2a)\nLocation:\n  light checkout root: /w/lw\n   checkout of branch: /w/trunk\n";
        let (t, info) = parse_bzr_info(lw);
        assert_eq!(bzr_kind_and_root(&t, &info), (BzrKind::Lightweight, "/w/lw".to_string()));
        assert_eq!(info.get("checkout_of_branch"), "/w/trunk");
    }

    #[test]
    fn bzr_info_ignores_lines_that_do_not_match_the_key_pattern() {
        let (_, info) = parse_bzr_info("Standalone tree\nsomething.dotted: x\n  ok key: v: w\nnocolon\n");
        assert!(!info.has("something.dotted"));
        // the value is everything after the first ": "
        assert_eq!(info.get("ok_key"), "v: w");
    }

    #[test]
    fn bzr_version_info_drops_empty_fields() {
        assert_eq!(parse_bzr_version_info("1234:trunk:1"), vec!["1234", "trunk", "1"]);
        assert_eq!(parse_bzr_version_info("1234::0"), vec!["1234", "0"]);
        assert!(parse_bzr_version_info("").is_empty());
    }

    #[test]
    fn bzr_stat_counts_flags_in_hash_order() {
        // bucket = hasher(key) % 17: "+N" = 1497 -> 1, "M" = 77 -> 9, "?" = 63 -> 12
        let out = " M  src/a.c\n M  src/b.c\n?   notes.txt\n+N  new.c\n\n";
        assert_eq!(parse_bzr_stat(out), "+N:1 M:2 ?:1 ");
        assert_eq!(parse_bzr_stat(""), "");
    }

    #[test]
    fn mtn_branch_is_text_after_the_last_separator() {
        let out = "Current branch: net.example.project\nChanges against parent 0123abcd:\n  patched  foo.c\n";
        assert_eq!(parse_mtn_branch(out), "net.example.project");
        assert_eq!(parse_mtn_branch("no branch here\n"), "");
        assert_eq!(parse_mtn_branch("Current branch:\n"), "Current branch:");
    }

    #[test]
    fn tla_branch_is_the_last_path_component() {
        assert_eq!(tla_branch("user@example.com--2005/proj--main--1.0\n"), "proj--main--1.0");
        assert_eq!(tla_branch("plain\n"), "plain");
    }

    #[test]
    fn cvs_branch_strips_the_directory_prefix() {
        assert_eq!(cvs_branch("proj/sub/dir\n", "proj"), "sub/dir");
        assert_eq!(cvs_branch("other/path\n", "proj"), "other/path");
        assert_eq!(cvs_branch("\n", "proj"), "proj");
        assert_eq!(cvs_branch("proj/\n", "proj"), "proj");
    }

    const FOSSIL_STATUS: &str = "repository:   /home/u/proj.fossil\n\
        local-root:   /home/u/proj/\n\
        config-db:    /home/u/.fossil\n\
        checkout:     0123456789abcdef0123456789abcdef01234567 2024-05-01 10:11:12 UTC\n\
        parent:       89abcdef0123456789abcdef0123456789abcdef 2024-04-30 09:00:00 UTC\n\
        tags:         trunk, release\n\
        comment:      fix thing (user: dev)\n\
        EDITED     src/a.c\n\
        ADDED      src/b.c\n";

    #[test]
    fn fossil_status_fields() {
        let s = parse_fossil_status(FOSSIL_STATUS);
        assert_eq!(s.hash, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(s.branch, "trunk");
        assert_eq!(s.local_root, "/home/u/proj/");
        assert_eq!(s.repository, "/home/u/proj.fossil");
        assert_eq!(s.action, "");
        // both change lines are keys; listed in zsh hash order
        let both = ["EDITED     src/a.c".to_string(), "ADDED      src/b.c".to_string()];
        assert_eq!(s.changed, zsh_hash_order(&both).join(" "));
        assert_eq!(zsh_tail(&s.local_root), "proj");
    }

    #[test]
    fn fossil_merge_marks_and_clean_tree() {
        let merging = "tags: trunk\nUPDATED_BY_MERGE src/m.c\n";
        let s = parse_fossil_status(merging);
        assert_eq!(s.action, "merging");
        assert_eq!(s.changed, "UPDATED_BY_MERGE src/m.c");
    }

    #[test]
    fn p4_server_defaults() {
        assert_eq!(p4_server_port("P4PORT=perforce.example.com:1666 (set)\n"), "perforce.example.com:1666");
        assert_eq!(p4_server_port("P4USER=me (set)\n"), "perforce:1666");
        assert_eq!(p4_server_port("P4PORT=:1999 (config)\n"), "perforce:1999");
        assert_eq!(p4_server_port("P4PORT=host: (set)\n"), "host:1666");
        assert_eq!(p4_server_port("P4PORT=1999 (set)\n"), "perforce:1999");
        assert_eq!(p4_server_port("P4PORT=: (set)\n"), "perforce:1666");
    }

    #[test]
    fn p4_change_number() {
        assert_eq!(
            parse_p4_change("Change 48213 on 2024/05/01 by dev@ws 'Fix build'\n"),
            "48213"
        );
        assert_eq!(parse_p4_change(""), "");
    }

    const SVK_CONFIG: &str = "---\n\
        contrib:\n\
        \x20 foo: bar\n\
        hash:\n\
        \x20 hash:\n\
        \x20   /home/u/proj: /mirror/trunk\n\
        \x20   /home/u/other: /mirror/other\n\
        \x20 depotpath: //mirror/trunk\n\
        \x20 revision: 1234\n\
        \x20 other: x\n";

    #[test]
    fn svk_config_finds_the_checkout_containing_pwd() {
        // The hash entry is `    /home/u/proj: ...` (4 spaces); depotpath/revision lines follow.
        let found = parse_svk_config(SVK_CONFIG, "/home/u/proj/sub");
        assert_eq!(
            found,
            Some(("/home/u/proj".to_string(), "trunk".to_string(), "1234".to_string()))
        );
        assert_eq!(parse_svk_config(SVK_CONFIG, "/elsewhere"), None);
    }

    #[test]
    fn svk_stops_at_the_next_section_header() {
        let cfg = "  hash:\n  next: y\n    /home/u/proj: x\n  depotpath: //a/b\n  revision: 9\n";
        // `  next: y` ends the hash block before any entry is read.
        assert_eq!(parse_svk_config(cfg, "/home/u/proj"), None);
    }
}
