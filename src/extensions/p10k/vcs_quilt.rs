//! quilt support of `vcs_info` — port of `Functions/VCS_Info/VCS_INFO_quilt`
//! (the `standalone` and `addon` modes and its `quilt-match`,
//! `quilt-standalone-detect`, `quilt-dirfind` helpers) and of
//! `VCS_INFO_patch2subject`.
//!
//! Both modes are gated by `zstyle -t :vcs_info:<vcs>.quilt-<mode>:<uc>:<rrn>
//! use-quilt`, which the theme leaves unset, so they run only when the user
//! enables them. The styles read from that context: `use-quilt`,
//! `quilt-standalone` (standalone only), `quilt-patch-dir`, `get-unapplied`,
//! `quiltcommand`, plus `patch-format` / `nopatch-format`.

use crate::extensions::p10k::vcs_hooks::{Assoc, ParamValue};
use crate::extensions::p10k::vcs_other::{bydir_detect, run_tool, tool_bin, Run};
use std::path::{Component, Path, PathBuf};

// ---------------------------------------------------------------------
// VCS_INFO_patch2subject
// ---------------------------------------------------------------------

/// How many leading lines `VCS_INFO_patch2subject` examines.
const PATCH_HEAD_LINES: usize = 10;

/// `---[^-]*` / `Index:*` — the start of the unified diff proper.
fn starts_diff(line: &str) -> bool {
    line.starts_with("Index:")
        || line.strip_prefix("---").is_some_and(|rest| rest.chars().next().is_some_and(|c| c != '-'))
}

/// `commit <40 hex digits>` and nothing else.
fn is_git_show_header(line: &str) -> bool {
    line.strip_prefix("commit ")
        .is_some_and(|h| h.len() == 40 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// `^r[0-9]* [|] .*` — the first line of an `svn log` entry.
fn is_svn_log_line(line: &str) -> bool {
    line.strip_prefix('r')
        .map(|r| r.trim_start_matches(|c: char| c.is_ascii_digit()))
        .is_some_and(|r| r.starts_with(" | "))
}

/// `read -r var` with the default `IFS`: leading and trailing blanks go.
fn read_split(line: &str) -> &str {
    line.trim_matches(|c| c == ' ' || c == '\t')
}

/// The single-line subject of the patch text `text`; `None` when
/// `VCS_INFO_patch2subject` leaves `REPLY` unset.
pub(crate) fn patch_subject_of(text: &str) -> Option<String> {
    let file_lines: Vec<&str> = text.split('\n').collect();
    // The first LIMIT lines, up to the first empty line or the diff.
    let mut lines: Vec<&str> = Vec::new();
    for i in 0..PATCH_HEAD_LINES {
        let line = file_lines.get(i).copied().unwrap_or("");
        if line.is_empty() || starts_diff(line) {
            break;
        }
        lines.push(line);
    }
    let strip_to_value = |l: &str| l.split_once(": ").map_or(l, |(_, v)| v).to_string();

    if let Some(at) = lines.iter().position(|l| l.starts_with("Subject:")) {
        // `Subject: foo`, with rfc822 whitespace unfolding.
        let mut reply = strip_to_value(lines[at]);
        if let Some(rest) = reply.strip_prefix("[PATCH] ") {
            reply = rest.to_string();
        }
        for cont in &lines[at + 1..] {
            if !cont.starts_with(' ') {
                break;
            }
            reply.push_str(cont);
        }
        return Some(reply);
    }
    if let Some(desc) = lines.iter().find(|l| l.starts_with("Description:")) {
        // DEP-3 `Description: foo`.
        return Some(strip_to_value(desc));
    }
    if lines.first() == Some(&"# HG changeset patch") {
        if let Some(first) = lines.iter().find(|l| !l.starts_with('#')) {
            return Some(first.to_string());
        }
    }
    if lines.len() == 3
        && is_git_show_header(lines[0])
        && lines[1].starts_with("Author:")
        && lines[2].starts_with("Date:")
    {
        // `git show` output: the log message follows the blank line after
        // `Date:`, and a second message line marks a longer message.
        let indent = "    ";
        let line = |i: usize| file_lines.get(i).copied().unwrap_or("");
        let subject = line(4).strip_prefix(indent).unwrap_or(line(4));
        let next = line(5).strip_prefix(indent).unwrap_or(line(5));
        let more = if next.is_empty() { "" } else { "..." };
        return Some(format!("{subject}{more}"));
    }
    if lines.first().is_some_and(|l| is_svn_log_line(l))
        || lines.get(1).is_some_and(|l| is_svn_log_line(l))
    {
        // Skip the header paragraph, take the first message line, and note
        // whether a second message line follows.
        let mut rest = file_lines.iter();
        for l in rest.by_ref() {
            if read_split(l).is_empty() {
                break;
            }
        }
        let subject = rest.next().map_or("", |l| read_split(l));
        let multiline = rest.next().is_some_and(|l| !l.is_empty());
        return Some(if multiline { format!("{subject}...") } else { subject.to_string() });
    }
    // The first line of the file is not part of the diff.
    lines.first().map(|l| l.to_string())
}

/// `VCS_INFO_patch2subject file`: `None` when `file` cannot be read (the
/// function fails) or has no subject (`REPLY` unset).
pub(crate) fn patch_subject(file: &Path) -> Option<String> {
    if !file.is_file() {
        return None;
    }
    let bytes = std::fs::read(file).ok()?;
    patch_subject_of(&String::from_utf8_lossy(&bytes))
}

// ---------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------

/// `${path:P}` for a path relative to `base`: each component is resolved
/// through symlinks while it exists, `..` pops the resolved path, and a
/// component that does not exist is kept as written.
fn realpath_in(base: &Path, path: &str) -> String {
    let mut cur = PathBuf::new();
    for comp in base.join(path).components() {
        match comp {
            Component::Prefix(_) | Component::RootDir => cur.push(comp),
            Component::CurDir => {}
            Component::ParentDir => {
                cur.pop();
            }
            Component::Normal(name) => {
                cur.push(name);
                if let Ok(real) = cur.canonicalize() {
                    cur = real;
                }
            }
        }
    }
    cur.to_string_lossy().into_owned()
}

/// `$(<file)`: the contents without trailing newlines; empty when unreadable.
fn read_substitution(file: &Path) -> String {
    let bytes = std::fs::read(file).unwrap_or_default();
    String::from_utf8_lossy(&bytes).trim_end_matches('\n').to_string()
}

/// `VCS_INFO_quilt-match`: the first `dir` of `list` that contains `pwd`
/// (`[[ $PWD == ${d%/##}(|/*) ]]`).
fn quilt_match(list: &[String], pwd: &str) -> Option<String> {
    list.iter()
        .find(|d| {
            let base = d.strip_suffix('/').unwrap_or(d);
            pwd == base || pwd.strip_prefix(base).is_some_and(|rest| rest.starts_with('/'))
        })
        .cloned()
}

/// `${(O)list}`: descending order.
fn sorted_descending(mut list: Vec<String>) -> Vec<String> {
    list.sort();
    list.reverse();
    list
}

// ---------------------------------------------------------------------
// VCS_INFO_quilt
// ---------------------------------------------------------------------

impl Run<'_> {
    /// `VCS_INFO_quilt-standalone-detect`: is the current directory under a
    /// quilt tree the user asked to be shown?
    fn quilt_standalone_detect(&mut self, ctx: &str) -> bool {
        let Some(param) = self.host.style_s(ctx, "quilt-standalone") else {
            return false;
        };
        match param.as_str() {
            "never" => return false,
            "always" => return true,
            _ => {}
        }
        if self.host.user_function_exists(&param) {
            return self.host.call_user_function(&param, &[], &mut self.st) == 0;
        }
        match self.host.param(&param) {
            ParamValue::Assoc(pairs) => {
                let keys = sorted_descending(pairs.iter().map(|(k, _)| k.clone()).collect());
                quilt_match(&keys, &self.pwd).is_some_and(|dir| {
                    pairs.iter().any(|(k, v)| *k == dir && v == "true")
                })
            }
            ParamValue::Array(items) => {
                quilt_match(&sorted_descending(items), &self.pwd).is_some()
            }
            ParamValue::Scalar(s) => s == "always",
            ParamValue::Unset => false,
        }
    }

    /// `VCS_INFO_quilt mode`: returns the exit status and the `REPLY` an
    /// add-on call leaves (the patch-format string; empty on failure).
    /// `mode` is `standalone` or `addon`.
    pub(crate) fn quilt(&mut self, mode: &str) -> (i32, String) {
        // The function declares `local -A hook_com`.
        self.with_hook_com(Assoc::default(), |r| r.quilt_in_scope(mode))
    }

    fn quilt_in_scope(&mut self, mode: &str) -> (i32, String) {
        let v = &self.st.vars;
        let ctx = format!(":vcs_info:{}.quilt-{mode}:{}:{}", v.vcs, v.usercontext, v.rrn);
        if !self.host.style_t(&ctx, "use-quilt") {
            return (1, String::new());
        }
        match mode {
            "standalone" => {
                if !self.quilt_standalone_detect(&ctx) {
                    return (1, String::new());
                }
            }
            "addon" => {}
            _ => return (2, String::new()),
        }

        // 1. A `.pc/.version` in a parent directory names the patches dir.
        // 2. Else the style, `$QUILT_PATCHES`, or `patches`.
        let standalone = self.st.vars.quiltmode == "standalone";
        let applied;
        let patches;
        let mut pc = String::new();
        let mut root = String::new();
        let mut quilt_patches_env = None;
        if let Some(dir) = bydir_detect(&self.cwd, ".pc", &[".version"]) {
            if standalone {
                root = dir.to_string_lossy().into_owned();
            }
            let pc_dir = dir.join(".pc");
            applied = if pc_dir.join("applied-patches").exists() {
                let mut list: Vec<String> = read_substitution(&pc_dir.join("applied-patches"))
                    .split('\n')
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect();
                list.reverse();
                list
            } else {
                Vec::new()
            };
            patches = realpath_in(&dir, &read_substitution(&pc_dir.join(".quilt_patches")));
            pc = pc_dir.to_string_lossy().into_owned();
        } else {
            applied = Vec::new();
            let configured = self
                .host
                .style_s(&ctx, "quilt-patch-dir")
                .or_else(|| match self.host.param("QUILT_PATCHES") {
                    ParamValue::Scalar(s) => Some(s),
                    _ => None,
                })
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| "patches".to_string());
            if configured.starts_with('/') {
                if !Path::new(&configured).is_dir() {
                    return (1, String::new());
                }
                patches = configured;
            } else {
                match bydir_detect(&self.cwd, &configured, &[]) {
                    Some(dir) => patches = format!("{}/{configured}", dir.display()),
                    None => return (1, String::new()),
                }
            }
            quilt_patches_env = Some(patches.clone());
        }

        let unapplied = if self.host.style_t(&ctx, "get-unapplied") {
            self.quilt_unapplied(&ctx, quilt_patches_env.as_deref())
        } else {
            Vec::new()
        };

        let with_subjects = |names: &[String]| -> Vec<String> {
            names
                .iter()
                .map(|name| {
                    match patch_subject(&Path::new(&patches).join(name)) {
                        Some(subject) => format!("{name} {subject}"),
                        None => format!("{name} ?"),
                    }
                })
                .collect()
        };
        let applied = with_subjects(&applied);
        let unapplied = with_subjects(&unapplied);

        let mut extra = Assoc::default();
        extra.set("quilt-patches-dir", patches.as_str());
        if !pc.is_empty() {
            extra.set("quilt-pc-dir", pc.as_str());
        }
        let qstring = self.set_patch_format_in(&applied, &unapplied, &ctx, &extra, &extra, &|_| Vec::new());

        let mut reply = String::new();
        if mode == "standalone" {
            self.st.backend_misc.set("patches", qstring.as_str());
            self.formats("", "", &root, "", "", "", &qstring);
        } else {
            reply = qstring;
        }
        let pc_arg = if pc.is_empty() { "\\-nopc-".to_string() } else { pc };
        let status = self.hook("post-quilt", &[mode.to_string(), patches.clone(), pc_arg]);
        (status, reply)
    }

    /// `quilt --quiltrc /dev/null unapplied`, run in the current directory.
    fn quilt_unapplied(&self, ctx: &str, patches_env: Option<&str>) -> Vec<String> {
        let command = self.host.style_s(ctx, "quiltcommand").unwrap_or_else(|| "quilt".into());
        let Some(bin) = tool_bin(&command) else { return Vec::new() };
        let env: Vec<(&str, &str)> = patches_env.map(|p| ("QUILT_PATCHES", p)).into_iter().collect();
        run_tool(&bin, &["--quiltrc", "/dev/null", "unapplied"], &self.cwd, &env)
            .map(|out| out.stdout.lines().filter(|l| !l.is_empty()).map(str::to_string).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::p10k::vcs_hooks::test_host::MockHost;
    use crate::extensions::p10k::vcs_hooks::HookHost;

    fn lines(s: &[&str]) -> String {
        s.join("\n")
    }

    #[test]
    fn subject_header_is_unfolded_and_patch_tag_dropped() {
        let text = lines(&[
            "From: A <a@example.com>",
            "Subject: [PATCH] fix the frobnicator",
            " across two lines",
            "",
            "body",
        ]);
        assert_eq!(
            patch_subject_of(&text).as_deref(),
            Some("fix the frobnicator across two lines")
        );
        // only the exact `[PATCH] ` tag is removed
        assert_eq!(
            patch_subject_of("Subject: [PATCH 1/2] x\n").as_deref(),
            Some("[PATCH 1/2] x")
        );
    }

    #[test]
    fn dep3_hg_and_first_line_subjects() {
        assert_eq!(
            patch_subject_of("Author: x\nDescription: backport foo\n---\n").as_deref(),
            Some("backport foo")
        );
        assert_eq!(
            patch_subject_of("# HG changeset patch\n# User x\nadd a thing\n\ndiff").as_deref(),
            Some("add a thing")
        );
        assert_eq!(patch_subject_of("plain first line\nsecond\n").as_deref(), Some("plain first line"));
        // the diff starts at once: no subject
        assert_eq!(patch_subject_of("--- a/f\n+++ b/f\n"), None);
        assert_eq!(patch_subject_of("Index: f\n"), None);
        // `---` followed by `-` is not a diff header
        assert_eq!(patch_subject_of("----\n").as_deref(), Some("----"));
    }

    #[test]
    fn git_show_subject_notes_a_longer_message() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        let one = lines(&[
            &format!("commit {hash}"),
            "Author: A <a@example.com>",
            "Date:   Mon Jan 1 00:00:00 2024 +0000",
            "",
            "    one line subject",
            "",
            "diff --git a/f b/f",
        ]);
        assert_eq!(patch_subject_of(&one).as_deref(), Some("one line subject"));
        let two = lines(&[
            &format!("commit {hash}"),
            "Author: A <a@example.com>",
            "Date:   Mon Jan 1 00:00:00 2024 +0000",
            "",
            "    subject",
            "    more text",
        ]);
        assert_eq!(patch_subject_of(&two).as_deref(), Some("subject..."));
        // a decorated `commit <hash> (HEAD)` line is not recognised, so the
        // first line stands in
        let decorated = lines(&[
            &format!("commit {hash} (HEAD)"),
            "Author: A",
            "Date: now",
            "",
            "    s",
        ]);
        assert_eq!(patch_subject_of(&decorated).as_deref(), Some(format!("commit {hash} (HEAD)").as_str()));
    }

    #[test]
    fn svn_log_subject_is_the_first_message_line() {
        let text = lines(&[
            "r42 | alice | 2024-01-01 00:00:00 +0000 (Mon, 01 Jan 2024) | 2 lines",
            "",
            "  fix the thing  ",
            "second line",
        ]);
        assert_eq!(patch_subject_of(&text).as_deref(), Some("fix the thing..."));
        let single = lines(&["r7 | bob | now | 1 line", "", "just this", ""]);
        assert_eq!(patch_subject_of(&single).as_deref(), Some("just this"));
        assert!(is_svn_log_line("r | x"));
        assert!(!is_svn_log_line("r12|x"));
        assert!(!is_svn_log_line("x12 | y"));
    }

    #[test]
    fn only_the_first_ten_lines_are_examined() {
        let mut text = String::new();
        for i in 0..12 {
            text.push_str(&format!("line {i}\n"));
        }
        text.push_str("Subject: too late\n");
        assert_eq!(patch_subject_of(&text).as_deref(), Some("line 0"));
    }

    #[test]
    fn patch_subject_reads_files_and_fails_on_missing() {
        let dir = std::env::temp_dir().join(format!("p10k-quilt-subject-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.patch"), "Subject: hello\n\n--- a\n").unwrap();
        assert_eq!(patch_subject(&dir.join("a.patch")).as_deref(), Some("hello"));
        assert_eq!(patch_subject(&dir.join("missing.patch")), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn quilt_match_takes_directory_prefixes_only() {
        let dirs = vec!["/src/proj/".to_string(), "/other".to_string()];
        assert_eq!(quilt_match(&dirs, "/src/proj").as_deref(), Some("/src/proj/"));
        assert_eq!(quilt_match(&dirs, "/src/proj/sub/dir").as_deref(), Some("/src/proj/"));
        assert_eq!(quilt_match(&dirs, "/src/project"), None);
        assert_eq!(quilt_match(&["/".to_string()], "/anywhere").as_deref(), Some("/"));
    }

    fn run_in<'a>(host: &'a dyn HookHost, cwd: &Path) -> Run<'a> {
        let mut run = Run::new(host, cwd.to_path_buf(), cwd.to_string_lossy().into_owned());
        run.st.vars.vcs = "-quilt-".into();
        run.st.vars.quiltmode = "standalone".into();
        run.st.vars.maxexports = 2;
        run
    }

    const CTX: &str = ":vcs_info:-quilt-.quilt-standalone:default:-all-";

    /// A quilt tree: `.pc/.version`, `.pc/.quilt_patches`, two applied
    /// patches and one patch file only on disk.
    fn quilt_tree(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("p10k-quilt-{tag}-{}", std::process::id()));
        let root = root.canonicalize().unwrap_or(root);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".pc")).unwrap();
        std::fs::create_dir_all(root.join("patches")).unwrap();
        std::fs::write(root.join(".pc/.version"), "2\n").unwrap();
        std::fs::write(root.join(".pc/.quilt_patches"), "patches\n").unwrap();
        std::fs::write(root.join(".pc/applied-patches"), "first.patch\nsecond.patch\n").unwrap();
        std::fs::write(root.join("patches/first.patch"), "Subject: one\n\n--- a\n").unwrap();
        std::fs::write(root.join("patches/second.patch"), "--- a/x\n+++ b/x\n").unwrap();
        let root = root.canonicalize().unwrap();
        root
    }

    #[test]
    fn standalone_requires_use_quilt_and_detection() {
        let root = quilt_tree("gate");
        // no use-quilt: off
        let host = MockHost::default();
        assert_eq!(run_in(&host, &root).quilt("standalone").0, 1);
        // use-quilt but no quilt-standalone: off
        let host = MockHost::default().with_style(CTX, "use-quilt", &["true"]);
        assert_eq!(run_in(&host, &root).quilt("standalone").0, 1);
        // never
        let host = MockHost::default()
            .with_style(CTX, "use-quilt", &["true"])
            .with_style(CTX, "quilt-standalone", &["never"]);
        assert_eq!(run_in(&host, &root).quilt("standalone").0, 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn standalone_always_formats_the_applied_patches() {
        let root = quilt_tree("always");
        let host = MockHost::default()
            .with_style(CTX, "use-quilt", &["true"])
            .with_style(CTX, "quilt-standalone", &["always"])
            .with_style(":vcs_info:-quilt-:default:-all-", "formats", &["Q:%m|%R"]);
        let mut run = run_in(&host, &root);
        let (status, reply) = run.quilt("standalone");
        assert_eq!(status, 0);
        assert!(reply.is_empty());
        // newest applied patch first, subjects appended, `?` without one
        assert_eq!(
            run.st.msgs,
            vec![format!("Q:second.patch ? (2 applied)|{}", root.display())]
        );
        // VCS_INFO_formats ends with `backend_misc=()`
        assert!(run.st.backend_misc.get("patches").is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn standalone_assoc_param_selects_by_directory() {
        let root = quilt_tree("assoc");
        let make = |value: &str| {
            let mut host = MockHost::default()
                .with_style(CTX, "use-quilt", &["true"])
                .with_style(CTX, "quilt-standalone", &["QUILT_DIRS"]);
            host.params.insert(
                "QUILT_DIRS".into(),
                ParamValue::Assoc(vec![
                    ("/".into(), "false".into()),
                    (format!("{}/", root.display()), value.into()),
                ]),
            );
            host
        };
        // the deepest (reverse-sorted first) matching key decides
        assert_eq!(run_in(&make("true"), &root).quilt("standalone").0, 0);
        assert_eq!(run_in(&make("false"), &root).quilt("standalone").0, 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn standalone_array_scalar_and_function_params() {
        let root = quilt_tree("kinds");
        let base = || {
            MockHost::default()
                .with_style(CTX, "use-quilt", &["true"])
                .with_style(CTX, "quilt-standalone", &["P"])
        };
        let mut h = base();
        h.params.insert("P".into(), ParamValue::Array(vec!["/nowhere".into(), root.display().to_string()]));
        assert_eq!(run_in(&h, &root).quilt("standalone").0, 0);
        let mut h = base();
        h.params.insert("P".into(), ParamValue::Array(vec!["/nowhere".into()]));
        assert_eq!(run_in(&h, &root).quilt("standalone").0, 1);
        let mut h = base();
        h.params.insert("P".into(), ParamValue::Scalar("always".into()));
        assert_eq!(run_in(&h, &root).quilt("standalone").0, 0);
        let mut h = base();
        h.params.insert("P".into(), ParamValue::Scalar("sometimes".into()));
        assert_eq!(run_in(&h, &root).quilt("standalone").0, 1);
        // a function of that name is called and its status decides
        let mut h = base();
        h.functions.push("P".into());
        assert_eq!(run_in(&h, &root).quilt("standalone").0, 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn addon_replies_with_the_patch_format_and_runs_post_quilt() {
        let root = quilt_tree("addon");
        let actx = ":vcs_info:git.quilt-addon:default:repo";
        let mut host = MockHost::default()
            .with_style(actx, "use-quilt", &["true"])
            .with_style(actx, "patch-format", &["%p/%n/%c/%a"])
            .with_style(":vcs_info:git+post-quilt:default:repo", "hooks", &["pq"]);
        host.functions.push("+vi-pq".into());
        let mut run = run_in(&host, &root);
        run.st.vars.vcs = "git".into();
        run.st.vars.quiltmode = "addon".into();
        run.st.vars.rrn = "repo".into();
        run.st.hook_com.set("branch", "outer");
        let (status, reply) = run.quilt("addon");
        assert_eq!((status, reply.as_str()), (0, "second.patch ?/2/0/2"));
        // the caller's hook_com is untouched by the quilt-local one
        assert_eq!(run.st.hook_com.get("branch"), "outer");
        let calls = host.calls.borrow();
        let pq = calls.iter().find(|(f, _)| f == "+vi-pq").expect("post-quilt hook ran");
        assert_eq!(pq.1[0], "addon");
        assert_eq!(pq.1[1], format!("{}/patches", root.display()));
        assert_eq!(pq.1[2], root.join(".pc").display().to_string());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn patch_dir_style_is_used_without_a_pc_dir() {
        let root = std::env::temp_dir().join(format!("p10k-quilt-nopc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::create_dir_all(root.join("mypatches")).unwrap();
        let root = root.canonicalize().unwrap();
        let actx = ":vcs_info:git.quilt-addon:default:-all-";
        let host = MockHost::default()
            .with_style(actx, "use-quilt", &["true"])
            .with_style(actx, "quilt-patch-dir", &["mypatches"])
            .with_style(actx, "nopatch-format", &["none here"]);
        let mut run = run_in(&host, &root.join("sub/deep"));
        run.st.vars.vcs = "git".into();
        run.st.vars.quiltmode = "addon".into();
        assert_eq!(run.quilt("addon"), (0, "none here".to_string()));
        // a patches directory that does not exist anywhere above: failure
        let host = MockHost::default()
            .with_style(actx, "use-quilt", &["true"])
            .with_style(actx, "quilt-patch-dir", &["absent"]);
        let mut run = run_in(&host, &root.join("sub/deep"));
        run.st.vars.vcs = "git".into();
        run.st.vars.quiltmode = "addon".into();
        assert_eq!(run.quilt("addon"), (1, String::new()));
        // an absolute one must exist
        let host = MockHost::default()
            .with_style(actx, "use-quilt", &["true"])
            .with_style(actx, "quilt-patch-dir", &["/definitely/not/there"]);
        let mut run = run_in(&host, &root);
        run.st.vars.vcs = "git".into();
        run.st.vars.quiltmode = "addon".into();
        assert_eq!(run.quilt("addon").0, 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn realpath_resolves_the_existing_part_and_folds_the_rest() {
        let root = std::env::temp_dir().join(format!("p10k-quilt-real-{}", std::process::id()));
        std::fs::create_dir_all(root.join("a")).unwrap();
        let root = root.canonicalize().unwrap();
        assert_eq!(realpath_in(&root, "a"), root.join("a").display().to_string());
        assert_eq!(realpath_in(&root, "a/../a/./x/y"), root.join("a/x/y").display().to_string());
        assert_eq!(realpath_in(&root, ""), root.display().to_string());
        std::fs::remove_dir_all(&root).ok();
    }
}
