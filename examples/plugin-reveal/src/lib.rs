//! **reveal** — open the current git repository's GitHub (and Heroku) pages
//! in the browser — ported to a native zshrs plugin. A faithful
//! reimplementation of MenkeTechnologies/gh_reveal's `reveal` function:
//!
//! - Not in a git repo, no arguments → open your GitHub repositories page
//!   (`$GITHUB_ACCOUNT`, else `git config user.name`).
//! - Not in a git repo, with arguments → treat each argument as a directory
//!   and reveal the repository there.
//! - In a git repo → parse `git remote -v`, optionally filtered by the
//!   arguments, and open the corresponding GitHub URL for each remote (Heroku
//!   remotes open their dashboard + app URLs).
//!
//! `// c:` citations refer to the `bin/reveal` shell function.

use std::os::raw::c_int;
use std::process::{Command, Stdio};
use znative::{declare_plugin, Args, Host};

/// The platform "open a URL" command, as argv. c:12-24.
fn open_cmd() -> Option<Vec<String>> {
    let v = |parts: &[&str]| Some(parts.iter().map(|s| s.to_string()).collect());
    match std::env::consts::OS {
        "macos" => v(&["open"]),
        "linux" => {
            // WSL exposes the Windows shell; native Linux uses xdg-open. c:16-18.
            let is_wsl = std::fs::read_to_string("/proc/version")
                .map(|s| s.to_lowercase().contains("microsoft"))
                .unwrap_or(false);
            if is_wsl {
                v(&["cmd.exe", "/c", "start", ""])
            } else {
                v(&["xdg-open"])
            }
        }
        "windows" => v(&["cmd", "/c", "start", ""]),
        _ => None,
    }
}

/// Spawn the opener on each URL, detached, output suppressed (c:xargs … open).
fn open_urls(open: &[String], urls: &[String]) {
    if urls.is_empty() || open.is_empty() {
        return;
    }
    let _ = Command::new(&open[0])
        .args(&open[1..])
        .args(urls)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// `git rev-parse --git-dir` succeeds → inside a work tree. c:29.
fn in_git_dir(dir: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `$GITHUB_ACCOUNT`, else `git config user.name`. c:50-54.
fn github_account(host: &Host) -> String {
    if let Some(a) = host.getvar("GITHUB_ACCOUNT").filter(|s| !s.is_empty()) {
        return a;
    }
    Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Raw `git remote -v` output for `dir`.
fn git_remotes(dir: &str) -> String {
    Command::new("git")
        .args(["remote", "-v"])
        .current_dir(dir)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Normalize a git remote URL to `host/user/repo` (no scheme, no `.git`, no
/// port) — the shared tail of gh_reveal's SSH (`@`) and HTTPS (`//`) sed
/// pipelines (c: the `grep '@' … sed 's@:@/@' | cut -c 2-` and `grep '//' …
/// cut -c 4-` branches, then `perl -pe 's@:\d+/@/@'` and `sed 's@.git$@@'`).
/// Returns None for URLs that don't look like `host/path`.
fn normalize_remote(url: &str) -> Option<String> {
    let mut s = url.trim();

    // Strip a scheme: `https://`, `ssh://`, `git://`, …
    let had_scheme = if let Some(i) = s.find("://") {
        s = &s[i + 3..];
        true
    } else {
        false
    };

    // Drop any `user@` userinfo → bare `host[:port]/path` or `host:path`.
    if let Some(at) = s.find('@') {
        s = &s[at + 1..];
    }

    // Scheme-less SSH shorthand (`git@host:user/repo`): the first `:` is the
    // path separator, not a port — turn it into `/`. Scheme URLs keep their
    // `:port`, which the port-stripper below removes.
    let owned;
    let mut host_path = if !had_scheme && s.contains(':') {
        owned = s.replacen(':', "/", 1);
        owned
    } else {
        s.to_string()
    };

    // Drop a `:port` right after the host: `host:22/user` → `host/user`.
    if let Some(colon) = host_path.find(':') {
        if let Some(slash) = host_path[colon..].find('/') {
            let port = &host_path[colon + 1..colon + slash];
            if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() {
                host_path.replace_range(colon..colon + slash, "");
            }
        }
    }

    let cleaned = host_path.trim_end_matches('/').trim_end_matches(".git");
    if cleaned.contains('/') && !cleaned.starts_with('/') {
        Some(cleaned.to_string())
    } else {
        None
    }
}

/// If the remote points at Heroku, return the app name (last path segment,
/// `.git` stripped). c: the `grep 'heroku'` branch.
fn heroku_app(normalized: &str) -> Option<String> {
    if !normalized.contains("heroku.com") {
        return None;
    }
    normalized
        .rsplit('/')
        .next()
        .map(|s| s.trim_end_matches(".git").to_string())
}

/// One `git remote -v` fetch line → `(name, url)`, else None.
fn parse_fetch_line(line: &str) -> Option<(&str, &str)> {
    if !line.contains("(fetch)") {
        return None;
    }
    let mut it = line.split_whitespace();
    let name = it.next()?;
    let url = it.next()?;
    Some((name, url))
}

/// Reveal the repository at `dir`. When `filters` is non-empty, only remotes
/// whose line matches one of the filter substrings are opened (gh_reveal's
/// `grep -E "$(args | join |)"`). c:74-92.
fn reveal_at(open: &[String], dir: &str, filters: &[String]) {
    let mut gh_urls = Vec::new();
    let mut heroku_urls = Vec::new();

    for line in git_remotes(dir).lines() {
        let Some((_, url)) = parse_fetch_line(line) else {
            continue;
        };
        if !filters.is_empty() && !filters.iter().any(|f| line.contains(f.as_str())) {
            continue;
        }
        let Some(norm) = normalize_remote(url) else {
            continue;
        };
        if let Some(app) = heroku_app(&norm) {
            heroku_urls.push(format!("https://dashboard.heroku.com/apps/{app}"));
            heroku_urls.push(format!("https://{app}.herokuapp.com"));
        } else {
            gh_urls.push(format!("https://{norm}"));
        }
    }
    open_urls(open, &heroku_urls);
    open_urls(open, &gh_urls);
}

/// The `reveal` command (c:reveal function body).
fn reveal(host: &Host, args: &Args) -> c_int {
    let Some(open) = open_cmd() else {
        host.print("reveal: unsupported platform\n");
        return 1;
    };
    let rest = args.rest();
    let here = in_git_dir(".");

    // c:60-64 — no git dir, no args: open your repositories page.
    if !here && rest.is_empty() {
        let name = github_account(host);
        if name.is_empty() {
            host.print("reveal: no git remote and no GITHUB_ACCOUNT / git user.name\n");
            return 1;
        }
        open_urls(
            &open,
            &[format!("https://github.com/{name}?tab=repositories")],
        );
        return 0;
    }

    // c:65-72 — no git dir, with args: reveal each argument directory.
    if !here && !rest.is_empty() {
        for dir in rest {
            if std::path::Path::new(dir).is_dir() {
                reveal_at(&open, dir, &[]);
            }
        }
        return 0;
    }

    // c:74-92 — inside a repo: open the remotes, filtered by the args.
    reveal_at(&open, ".", rest);
    0
}

declare_plugin! {
    name: "reveal",
    version: "0.1.0",
    builtins: {
        "reveal" => reveal,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ssh_shorthand() {
        assert_eq!(
            normalize_remote("git@github.com:MenkeTechnologies/zshrs.git").as_deref(),
            Some("github.com/MenkeTechnologies/zshrs")
        );
    }

    #[test]
    fn normalize_https() {
        assert_eq!(
            normalize_remote("https://github.com/MenkeTechnologies/zshrs.git").as_deref(),
            Some("github.com/MenkeTechnologies/zshrs")
        );
        // No `.git` suffix.
        assert_eq!(
            normalize_remote("https://gitlab.com/group/proj").as_deref(),
            Some("gitlab.com/group/proj")
        );
    }

    #[test]
    fn normalize_ssh_scheme_with_port() {
        assert_eq!(
            normalize_remote("ssh://git@github.com:22/user/repo.git").as_deref(),
            Some("github.com/user/repo")
        );
    }

    #[test]
    fn normalize_rejects_garbage() {
        assert_eq!(normalize_remote("not a url"), None);
        assert_eq!(normalize_remote(""), None);
    }

    #[test]
    fn heroku_detection() {
        let n = normalize_remote("https://git.heroku.com/my-app.git").unwrap();
        assert_eq!(heroku_app(&n).as_deref(), Some("my-app"));
        let gh = normalize_remote("git@github.com:u/r.git").unwrap();
        assert_eq!(heroku_app(&gh), None);
    }

    #[test]
    fn parse_fetch_line_only_fetch() {
        assert_eq!(
            parse_fetch_line("origin\tgit@github.com:u/r.git (fetch)"),
            Some(("origin", "git@github.com:u/r.git"))
        );
        assert_eq!(
            parse_fetch_line("origin\tgit@github.com:u/r.git (push)"),
            None
        );
        assert_eq!(parse_fetch_line(""), None);
    }

    #[test]
    fn open_cmd_is_known_on_this_platform() {
        // macOS and Linux (the supported targets) both resolve an opener.
        if matches!(std::env::consts::OS, "macos" | "linux" | "windows") {
            assert!(open_cmd().is_some());
        }
    }
}
