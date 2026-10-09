//! git configuration reader for the native git status path.
//!
//! Port of the parts of git's `config.c` the status code depends on, which
//! is also what libgit2 (gitstatusd's backend) implements:
//!
//! - the file grammar: `[section]`, `[section "sub"]`, the legacy
//!   `[section.sub]` (subsection lower-cased), keys on the header line,
//!   `=`-less keys (implicit boolean true), quoted values, the escapes
//!   `\n \t \b \\ \"`, backslash-newline continuation, and `#` / `;`
//!   comments outside quotes;
//! - `include.path` and `includeIf.<condition>.path` (depth limit 10):
//!   `gitdir:`, `gitdir/i:`, `onbranch:` and `hasconfig:remote.*.url:`;
//! - the file stack of a repository, lowest to highest priority: system,
//!   XDG, global, the repository's `config`, then `config.worktree` when
//!   `extensions.worktreeConfig` is on.
//!
//! The value of a key is the LAST one assigned (git-config `--get`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// `MAX_INCLUDE_DEPTH` (config.c).
const MAX_INCLUDE_DEPTH: usize = 10;

/// What conditional includes are evaluated against.
#[derive(Clone, Default)]
pub(crate) struct IncludeCtx {
    /// The repository's `$GIT_DIR` (`gitdir:` conditions).
    pub git_dir: PathBuf,
    /// The checked-out branch, short name (`onbranch:` conditions).
    pub branch: Option<String>,
    /// `$HOME`, for `~/` in include paths and patterns.
    pub home: Option<PathBuf>,
    /// Every `remote.*.url` seen, for `hasconfig:remote.*.url:`.
    remote_urls: Vec<String>,
    /// First pass over the stack: collect URLs, skip `hasconfig:` includes.
    collecting_urls: bool,
}

/// A parsed configuration: `section\0subsection\0key` -> last value.
/// `None` is a key written without `=` (implicit true).
#[derive(Default)]
pub(crate) struct GitConfig {
    map: HashMap<String, Option<String>>,
    remote_urls: Vec<String>,
    /// Every value assigned to a key, in order (multi-valued keys such as
    /// `remote.<name>.fetch`).
    all: HashMap<String, Vec<String>>,
    /// `section\0subsection` of every key seen.
    groups: HashSet<String>,
}

impl GitConfig {
    /// Parse `text` with no include context (includes need a file).
    #[cfg(test)]
    pub(crate) fn parse(text: &str) -> GitConfig {
        let mut cfg = GitConfig::default();
        cfg.parse_into(text, None, &IncludeCtx::default(), 0);
        cfg
    }

    /// `section.subsection.key`; `None` when unset or written without `=`.
    pub(crate) fn get(&self, section: &str, subsection: &str, key: &str) -> Option<&str> {
        self.map
            .get(&key_of(section, subsection, key))
            .and_then(|v| v.as_deref())
    }

    /// Every value of a multi-valued key, in assignment order.
    pub(crate) fn get_all(&self, section: &str, subsection: &str, key: &str) -> &[String] {
        self.all
            .get(&key_of(section, subsection, key))
            .map_or(&[], Vec::as_slice)
    }

    /// Whether any key of `[section "subsection"]` is set.
    pub(crate) fn has_group(&self, section: &str, subsection: &str) -> bool {
        self.groups.contains(&format!("{}\0{subsection}", section.to_ascii_lowercase()))
    }

    /// The boolean value of `section.key`; `None` when unset or not a
    /// boolean (`git_config_get_bool` failing).
    pub(crate) fn bool_value(&self, section: &str, key: &str) -> Option<bool> {
        match self.map.get(&key_of(section, "", key)) {
            None => None,
            Some(None) => Some(true),
            Some(Some(v)) => parse_bool(v),
        }
    }

    /// git-config `--type=bool`: `true/yes/on`, `false/no/off`, the empty
    /// string (false), or an integer (non-zero is true); a key without `=`
    /// is true. Unset or malformed gives `default`.
    pub(crate) fn get_bool(&self, section: &str, key: &str, default: bool) -> bool {
        match self.map.get(&key_of(section, "", key)) {
            None => default,
            Some(None) => true,
            Some(Some(v)) => parse_bool(v).unwrap_or(default),
        }
    }

    /// The configuration of a repository with the given directories, as
    /// libgit2 layers it. `branch` is the checked-out branch, if any.
    pub(crate) fn load_stack(
        common_dir: &Path,
        git_dir: &Path,
        branch: Option<&str>,
    ) -> GitConfig {
        let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
        let mut ctx = IncludeCtx {
            git_dir: git_dir.to_path_buf(),
            branch: branch.map(str::to_string),
            home: home.clone(),
            ..IncludeCtx::default()
        };
        let mut files = stack_files(home.as_deref(), &common_dir.join("config"));
        // `hasconfig:` looks at every URL in the whole stack, so collect
        // them before evaluating the conditions.
        let mut first = GitConfig::default();
        ctx.collecting_urls = true;
        first.read_files(&files, &ctx);
        ctx.collecting_urls = false;
        ctx.remote_urls = first.remote_urls;

        let mut cfg = GitConfig::default();
        cfg.read_files(&files, &ctx);
        if cfg.get_bool("extensions", "worktreeconfig", false) {
            files.clear();
            files.push(git_dir.join("config.worktree"));
            cfg.read_files(&files, &ctx);
        }
        cfg
    }

    fn read_files(&mut self, files: &[PathBuf], ctx: &IncludeCtx) {
        for file in files {
            self.read_file(file, ctx, 0);
        }
    }

    fn read_file(&mut self, file: &Path, ctx: &IncludeCtx, depth: usize) {
        let Ok(bytes) = std::fs::read(file) else { return };
        let text = String::from_utf8_lossy(&bytes);
        self.parse_into(&text, file.parent(), ctx, depth);
    }

    fn set(&mut self, section: &str, subsection: &str, key: &str, value: Option<String>) {
        let group = format!("{}\0{subsection}", section.to_ascii_lowercase());
        self.groups.insert(group);
        if let Some(v) = &value {
            self.all.entry(key_of(section, subsection, key)).or_default().push(v.clone());
        }
        if section == "remote" && key == "url" {
            if let Some(v) = &value {
                self.remote_urls.push(v.clone());
            }
        }
        self.map.insert(key_of(section, subsection, key), value);
    }

    /// Parse `text`; `dir` is the directory of the file it came from, which
    /// relative include paths are resolved against.
    fn parse_into(&mut self, text: &str, dir: Option<&Path>, ctx: &IncludeCtx, depth: usize) {
        let b = text.as_bytes();
        let (mut section, mut subsection) = (String::new(), String::new());
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            if c.is_ascii_whitespace() {
                i += 1;
            } else if c == b'#' || c == b';' {
                i = skip_line(b, i);
            } else if c == b'[' {
                match parse_header(b, i + 1) {
                    Some((sect, sub, next)) => {
                        section = sect;
                        subsection = sub;
                        i = next;
                    }
                    None => i = skip_line(b, i),
                }
            } else if c.is_ascii_alphabetic() && !section.is_empty() {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') {
                    i += 1;
                }
                let key = text[start..i].to_ascii_lowercase();
                while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
                    i += 1;
                }
                let value = if b.get(i) == Some(&b'=') {
                    match parse_value(b, i + 1) {
                        Some((v, next)) => {
                            i = next;
                            Some(v)
                        }
                        None => {
                            i = skip_line(b, i);
                            continue;
                        }
                    }
                } else {
                    i = skip_line(b, i);
                    None
                };
                self.set(&section, &subsection, &key, value.clone());
                if let Some(path) = value {
                    self.maybe_include(&section, &subsection, &key, &path, dir, ctx, depth);
                }
            } else {
                // Not a key, header or comment: git rejects the file here;
                // drop the line and carry on.
                i = skip_line(b, i);
            }
        }
    }

    /// `include.path` and a matching `includeIf.<cond>.path`.
    #[allow(clippy::too_many_arguments)]
    fn maybe_include(
        &mut self,
        section: &str,
        subsection: &str,
        key: &str,
        path: &str,
        dir: Option<&Path>,
        ctx: &IncludeCtx,
        depth: usize,
    ) {
        if key != "path" || depth + 1 >= MAX_INCLUDE_DEPTH {
            return;
        }
        let wanted = match section {
            "include" => subsection.is_empty(),
            "includeif" => condition_holds(subsection, dir, ctx),
            _ => false,
        };
        if !wanted {
            return;
        }
        let Some(target) = expand_path(path, ctx.home.as_deref()).map(|p| match dir {
            Some(d) if p.is_relative() => d.join(p),
            _ => p,
        }) else {
            return;
        };
        if target.is_absolute() {
            self.read_file(&target, ctx, depth + 1);
        }
    }
}

fn key_of(section: &str, subsection: &str, key: &str) -> String {
    format!("{}\0{subsection}\0{}", section.to_ascii_lowercase(), key.to_ascii_lowercase())
}

/// git `git_parse_maybe_bool`.
fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Some(true),
        "false" | "no" | "off" | "" => Some(false),
        other => {
            let digits = other.trim_end_matches(['k', 'm', 'g']);
            digits.parse::<i64>().ok().map(|n| n != 0)
        }
    }
}

/// The configuration files of a repository, lowest priority first.
fn stack_files(home: Option<&Path>, local: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if std::env::var_os("GIT_CONFIG_NOSYSTEM").is_none() {
        files.push(
            std::env::var_os("GIT_CONFIG_SYSTEM")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/etc/gitconfig")),
        );
    }
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|x| !x.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".config")));
    files.extend(xdg.map(|x| x.join("git/config")));
    files.extend(
        std::env::var_os("GIT_CONFIG_GLOBAL")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".gitconfig"))),
    );
    files.push(local.to_path_buf());
    files
}

/// `~/` and absolute paths; `None` for `~user` forms.
fn expand_path(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix("~/") {
        Some(rest) => home.map(|h| h.join(rest)),
        None if path.starts_with('~') => None,
        None => Some(PathBuf::from(path)),
    }
}

fn skip_line(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

/// A header after its `[`: `(section, subsection, index after "]")`.
fn parse_header(b: &[u8], mut i: usize) -> Option<(String, String, usize)> {
    let start = i;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b'.') {
        i += 1;
    }
    let name = String::from_utf8_lossy(&b[start..i]).to_ascii_lowercase();
    match b.get(i)? {
        b']' => {
            // `[section.sub]`: the subsection is lower-cased, like the section.
            Some(match name.split_once('.') {
                Some((s, sub)) => (s.to_string(), sub.to_string(), i + 1),
                None => (name, String::new(), i + 1),
            })
        }
        c if c.is_ascii_whitespace() => {
            while b.get(i).is_some_and(|c| *c == b' ' || *c == b'\t') {
                i += 1;
            }
            if b.get(i) != Some(&b'"') {
                return None;
            }
            i += 1;
            let mut sub = Vec::new();
            loop {
                match *b.get(i)? {
                    b'"' => break,
                    b'\n' => return None,
                    b'\\' => {
                        i += 1;
                        sub.push(*b.get(i).filter(|c| **c != b'\n')?);
                    }
                    c => sub.push(c),
                }
                i += 1;
            }
            (b.get(i + 1) == Some(&b']'))
                .then(|| (name, String::from_utf8_lossy(&sub).into_owned(), i + 2))
        }
        _ => None,
    }
}

/// A value after its `=` (git `parse_value`): `(value, index of the end of
/// the value's last line)`. `None` on a malformed value (bad escape, an
/// unterminated quote).
fn parse_value(b: &[u8], mut i: usize) -> Option<(String, usize)> {
    let mut out: Vec<u8> = Vec::new();
    let (mut quoted, mut pending_space) = (false, 0usize);
    while i < b.len() {
        let c = b[i];
        i += 1;
        match c {
            b'\n' => {
                if quoted {
                    return None;
                }
                i -= 1;
                break;
            }
            b'\\' => {
                let escaped = match *b.get(i)? {
                    b'\n' => {
                        i += 1;
                        continue;
                    }
                    b'\r' if b.get(i + 1) == Some(&b'\n') => {
                        i += 2;
                        continue;
                    }
                    b't' => b'\t',
                    b'b' => 0x08,
                    b'n' => b'\n',
                    c @ (b'\\' | b'"') => c,
                    _ => return None,
                };
                i += 1;
                out.extend(std::iter::repeat(b' ').take(std::mem::take(&mut pending_space)));
                out.push(escaped);
            }
            b'"' => quoted = !quoted,
            b';' | b'#' if !quoted => {
                i = skip_line(b, i);
                break;
            }
            c if c.is_ascii_whitespace() && !quoted => {
                if !out.is_empty() {
                    pending_space += 1;
                }
            }
            c => {
                out.extend(std::iter::repeat(b' ').take(std::mem::take(&mut pending_space)));
                out.push(c);
            }
        }
    }
    if quoted {
        return None;
    }
    Some((String::from_utf8_lossy(&out).into_owned(), i))
}

// ---------------------------------------------------------------------
// includeIf conditions
// ---------------------------------------------------------------------

/// The condition of `[includeIf "<subsection>"]`.
fn condition_holds(cond: &str, dir: Option<&Path>, ctx: &IncludeCtx) -> bool {
    if let Some(pat) = cond.strip_prefix("gitdir:") {
        return gitdir_matches(pat, false, dir, ctx);
    }
    if let Some(pat) = cond.strip_prefix("gitdir/i:") {
        return gitdir_matches(pat, true, dir, ctx);
    }
    if let Some(pat) = cond.strip_prefix("onbranch:") {
        let mut pattern = pat.to_string();
        if pattern.ends_with('/') {
            pattern.push_str("**");
        }
        return ctx
            .branch
            .as_deref()
            .is_some_and(|b| wildmatch(pattern.as_bytes(), b.as_bytes(), true, false));
    }
    if let Some(pat) = cond.strip_prefix("hasconfig:remote.*.url:") {
        return !ctx.collecting_urls
            && ctx.remote_urls.iter().any(|u| wildmatch(pat.as_bytes(), u.as_bytes(), false, false));
    }
    false
}

/// `include_by_gitdir`: the pattern is matched against the real path of
/// `$GIT_DIR` with `*` not crossing `/`.
fn gitdir_matches(pat: &str, fold: bool, dir: Option<&Path>, ctx: &IncludeCtx) -> bool {
    let mut pattern = if let Some(rest) = pat.strip_prefix("~/") {
        match &ctx.home {
            Some(h) => format!("{}/{rest}", h.display()),
            None => return false,
        }
    } else if let Some(rest) = pat.strip_prefix("./") {
        match dir {
            Some(d) => format!("{}/{rest}", d.display()),
            None => return false,
        }
    } else if pat.starts_with('/') {
        pat.to_string()
    } else {
        format!("**/{pat}")
    };
    if pattern.ends_with('/') {
        pattern.push_str("**");
    }
    let real = ctx.git_dir.canonicalize().unwrap_or_else(|_| ctx.git_dir.clone());
    [real.as_path(), ctx.git_dir.as_path()]
        .iter()
        .any(|p| wildmatch(pattern.as_bytes(), p.to_string_lossy().as_bytes(), true, fold))
}

/// git `wildmatch`: `?`, `*`, `[...]`, `\x`; with `pathname`, `*` and `?`
/// stop at `/` and `**` spans directories only as a whole path component
/// (`**/`, `/**`, `/**/`).
pub(crate) fn wildmatch(pat: &[u8], text: &[u8], pathname: bool, fold: bool) -> bool {
    let same = |a: u8, b: u8| if fold { a.eq_ignore_ascii_case(&b) } else { a == b };
    let (mut pi, mut ti) = (0, 0);
    while pi < pat.len() {
        match pat[pi] {
            b'?' => {
                if ti >= text.len() || (pathname && text[ti] == b'/') {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            b'\\' => {
                pi += 1;
                match (pat.get(pi), text.get(ti)) {
                    (Some(&p), Some(&t)) if same(p, t) => {
                        pi += 1;
                        ti += 1;
                    }
                    _ => return false,
                }
            }
            b'[' => {
                let Some(&t) = text.get(ti) else { return false };
                if pathname && t == b'/' {
                    return false;
                }
                let Some((hit, next)) = match_class(pat, pi + 1, t, fold) else {
                    return false;
                };
                if !hit {
                    return false;
                }
                pi = next;
                ti += 1;
            }
            b'*' => {
                let start = pi;
                while pat.get(pi) == Some(&b'*') {
                    pi += 1;
                }
                let mut spans_dirs = !pathname;
                if pathname && pi - start >= 2 {
                    let whole_component = (start == 0 || pat[start - 1] == b'/')
                        && (pi == pat.len() || pat[pi] == b'/');
                    if whole_component {
                        // `**/` may also match no directory at all.
                        if pat.get(pi) == Some(&b'/')
                            && wildmatch(&pat[pi + 1..], &text[ti..], pathname, fold)
                        {
                            return true;
                        }
                        spans_dirs = true;
                    }
                }
                let rest = &pat[pi..];
                if rest.is_empty() {
                    return spans_dirs || !text[ti..].contains(&b'/');
                }
                let mut k = ti;
                loop {
                    if wildmatch(rest, &text[k..], pathname, fold) {
                        return true;
                    }
                    if k >= text.len() || (!spans_dirs && text[k] == b'/') {
                        return false;
                    }
                    k += 1;
                }
            }
            c => match text.get(ti) {
                Some(&t) if same(c, t) => {
                    pi += 1;
                    ti += 1;
                }
                _ => return false,
            },
        }
    }
    ti == text.len()
}

/// One `[...]` class starting after the `[`: `(matched, index after "]")`.
fn match_class(pat: &[u8], mut pi: usize, t: u8, fold: bool) -> Option<(bool, usize)> {
    let negate = matches!(pat.get(pi), Some(b'!' | b'^'));
    if negate {
        pi += 1;
    }
    let norm = |c: u8| if fold { c.to_ascii_lowercase() } else { c };
    let t = norm(t);
    let (mut hit, mut first) = (false, true);
    loop {
        let mut c = *pat.get(pi)?;
        if c == b']' && !first {
            break;
        }
        first = false;
        if c == b'\\' {
            pi += 1;
            c = *pat.get(pi)?;
        }
        pi += 1;
        if pat.get(pi) == Some(&b'-') && pat.get(pi + 1).is_some_and(|e| *e != b']') {
            let mut hi = pat[pi + 1];
            pi += 2;
            if hi == b'\\' {
                hi = *pat.get(pi)?;
                pi += 1;
            }
            hit |= (norm(c)..=norm(hi)).contains(&t);
        } else {
            hit |= norm(c) == t;
        }
    }
    Some((hit != negate, pi + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("p10k-gitconfig-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn sections_keys_and_case_rules() {
        let cfg = GitConfig::parse(
            "# comment\n\
             [Core]\n\
             \tFileMode = false ; trailing comment\n\
             [branch \"Feature/X\"]\n\
             \tRemote = origin\n\
             [legacy.SubName]\n\
             \tkey = v\n\
             [one] two = 2\n",
        );
        assert_eq!(cfg.get("core", "", "filemode"), Some("false"));
        // subsections are case-sensitive, sections and keys are not
        assert_eq!(cfg.get("branch", "Feature/X", "remote"), Some("origin"));
        assert_eq!(cfg.get("branch", "feature/x", "remote"), None);
        // the old `[section.sub]` form lower-cases the subsection
        assert_eq!(cfg.get("legacy", "subname", "key"), Some("v"));
        // a key on the header line
        assert_eq!(cfg.get("one", "", "two"), Some("2"));
    }

    #[test]
    fn values_quotes_escapes_and_continuations() {
        let cfg = GitConfig::parse(
            "[s]\n\
             a = \"quoted ; not a comment\"\n\
             b = x \\\"y\\\" \\t z\n\
             c = one \\\n\
             two\n\
             d =   spaced   out   \n\
             e = \"  keep  \" tail # c\n\
             f = back\\\\slash\n",
        );
        assert_eq!(cfg.get("s", "", "a"), Some("quoted ; not a comment"));
        assert_eq!(cfg.get("s", "", "b"), Some("x \"y\" \t z"));
        assert_eq!(cfg.get("s", "", "c"), Some("one two"));
        assert_eq!(cfg.get("s", "", "d"), Some("spaced   out"));
        assert_eq!(cfg.get("s", "", "e"), Some("  keep   tail"));
        assert_eq!(cfg.get("s", "", "f"), Some("back\\slash"));
    }

    #[test]
    fn last_assignment_wins_and_bools_follow_git() {
        let cfg = GitConfig::parse(
            "[core]\n\
             filemode = true\n\
             filemode = no\n\
             symlinks\n\
             bare = 2\n\
             x = maybe\n\
             y =\n",
        );
        assert!(!cfg.get_bool("core", "filemode", true));
        assert!(cfg.get_bool("core", "symlinks", false), "bare key is true");
        assert!(cfg.get_bool("core", "bare", false), "non-zero integer");
        assert!(cfg.get_bool("core", "x", true), "malformed keeps the default");
        assert!(!cfg.get_bool("core", "y", true), "empty string is false");
        assert!(cfg.get_bool("core", "unset", true));
        // a key written without `=` has no string value
        assert_eq!(cfg.get("core", "", "symlinks"), None);
    }

    #[test]
    fn a_malformed_value_is_dropped_and_parsing_continues() {
        let cfg = GitConfig::parse("[s]\nbad = \\q\nunterminated = \"abc\ngood = 1\n");
        assert_eq!(cfg.get("s", "", "bad"), None);
        assert_eq!(cfg.get("s", "", "unterminated"), None);
        assert_eq!(cfg.get("s", "", "good"), Some("1"));
    }

    #[test]
    fn wildmatch_follows_git_semantics() {
        let m = |p: &str, t: &str| wildmatch(p.as_bytes(), t.as_bytes(), true, false);
        assert!(m("main", "main"));
        assert!(m("feature/*", "feature/x"));
        assert!(!m("feature/*", "feature/x/y"), "* stops at /");
        assert!(m("feature/**", "feature/x/y"));
        assert!(m("a/**/b", "a/b"));
        assert!(m("a/**/b", "a/x/y/b"));
        assert!(m("**/foo/.git", "/home/u/foo/.git"));
        assert!(!m("**/foo/.git", "/home/u/xfoo/.git"));
        assert!(m("rel-[0-9]?", "rel-4a"));
        assert!(!m("rel-[!0-9]", "rel-4"));
        assert!(m("a\\*b", "a*b"));
        assert!(wildmatch(b"*.example.com", b"git@x.example.com", false, false));
        assert!(wildmatch(b"https://*", b"https://h/a/b", false, false));
        assert!(wildmatch(b"/Work/**", b"/work/p/.git", true, true), "gitdir/i folds case");
    }

    #[test]
    fn include_paths_resolve_against_the_including_file() {
        let d = temp("include");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/extra"), "[core]\n\tsymlinks = false\n").unwrap();
        std::fs::write(d.join("config"), "[core]\n\tsymlinks = true\n[include]\n\tpath = sub/extra\n").unwrap();
        let mut cfg = GitConfig::default();
        cfg.read_file(&d.join("config"), &IncludeCtx::default(), 0);
        // the include is processed at its position, so it overrides
        assert!(!cfg.get_bool("core", "symlinks", true));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn include_cycles_stop_at_the_depth_limit() {
        let d = temp("cycle");
        std::fs::write(d.join("a"), "[include]\n\tpath = b\n[n]\n\tx = a\n").unwrap();
        std::fs::write(d.join("b"), "[include]\n\tpath = a\n[n]\n\tx = b\n").unwrap();
        let mut cfg = GitConfig::default();
        cfg.read_file(&d.join("a"), &IncludeCtx::default(), 0);
        assert!(cfg.get("n", "", "x").is_some());
        std::fs::remove_dir_all(&d).ok();
    }

    fn cond_cfg(d: &Path, text: &str, ctx: &IncludeCtx) -> GitConfig {
        std::fs::write(d.join("inc"), "[core]\n\tfilemode = false\n").unwrap();
        std::fs::write(d.join("config"), text).unwrap();
        let mut cfg = GitConfig::default();
        cfg.read_file(&d.join("config"), ctx, 0);
        cfg
    }

    #[test]
    fn include_if_gitdir_and_onbranch() {
        let d = temp("cond");
        let git_dir = d.join("work/proj/.git");
        std::fs::create_dir_all(&git_dir).unwrap();
        let ctx = IncludeCtx {
            git_dir: git_dir.clone(),
            branch: Some("release/1.2".into()),
            home: Some(d.clone()),
            ..IncludeCtx::default()
        };
        let on = |cond: &str| {
            let text = format!("[includeIf \"{cond}\"]\n\tpath = {}/inc\n", d.display());
            !cond_cfg(&d, &text, &ctx).get_bool("core", "filemode", true)
        };
        assert!(on("gitdir:~/work/"));
        assert!(on("gitdir:~/work/proj/.git"));
        assert!(on("gitdir:proj/.git"), "a relative pattern gets **/ in front");
        assert!(!on("gitdir:~/other/"));
        assert!(!on("gitdir:~/WORK/"));
        assert!(on("gitdir/i:~/WORK/"));
        assert!(on("onbranch:release/**"));
        assert!(!on("onbranch:main"));
        assert!(on("onbranch:release/1.2"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn has_config_matches_remote_urls_of_the_whole_stack() {
        let d = temp("hasconfig");
        let git_dir = d.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(d.join("inc"), "[core]\n\tfilemode = false\n").unwrap();
        // the URL comes AFTER the include that depends on it
        std::fs::write(
            git_dir.join("config"),
            format!(
                "[includeIf \"hasconfig:remote.*.url:https://corp.example/**\"]\n\tpath = {}/inc\n\
                 [remote \"origin\"]\n\turl = https://corp.example/team/repo.git\n",
                d.display()
            ),
        )
        .unwrap();
        let mut first = GitConfig::default();
        let mut ctx = IncludeCtx { git_dir: git_dir.clone(), collecting_urls: true, ..IncludeCtx::default() };
        first.read_file(&git_dir.join("config"), &ctx, 0);
        assert_eq!(first.remote_urls, vec!["https://corp.example/team/repo.git"]);
        assert!(first.get_bool("core", "filemode", true), "not evaluated while collecting");
        ctx.collecting_urls = false;
        ctx.remote_urls = first.remote_urls;
        let mut cfg = GitConfig::default();
        cfg.read_file(&git_dir.join("config"), &ctx, 0);
        assert!(!cfg.get_bool("core", "filemode", true));
        std::fs::remove_dir_all(&d).ok();
    }
}
