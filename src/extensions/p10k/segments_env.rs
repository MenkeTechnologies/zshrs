//! p10k VERSION-MANAGER / environment segments — Rust port of the
//! `prompt_*` functions from powerlevel10k `internal/p10k.zsh` (the
//! SPEC; every ported line is cited as `// p10k:NNN`).
//!
//! Covers the env/tool segments in the user's RIGHT_PROMPT_ELEMENTS:
//! direnv, asdf, virtualenv, anaconda, pyenv, goenv, nodenv, nvm,
//! nodeenv, node_version, go_version, rust_version, os_icon,
//! java_version, package, rbenv, rvm, fvm, luaenv, jenv, plenv,
//! phpenv, scalaenv, haskell_stack, kubecontext, terraform, aws,
//! aws_eb_env, azure, gcloud, google_app_cred, nordvpn, ranger, nnn.
//!
//! Show/hide fidelity is the contract: a segment returns
//! `Some(vec![])` (handled, hidden) whenever the zsh original would
//! `return` without calling `_p9k_prompt_segment`, and `None` only
//! for names this module does not own.
//!
//! External tools (node, go, rustc, java, kubectl, ...) are run via
//! `std::process::Command` behind a stat-keyed cache that mirrors
//! p10k's `_p9k_cache_stat_get`/`_p9k_cache_stat_set` (p10k caches on
//! the mtimes of the binary + a witness file, so a tool is re-run only
//! when something relevant changed — never on every prompt).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use crate::extensions::p10k::config::{p9k_global, p9k_param, p9k_param_arr, p9k_param_bool};
use crate::extensions::p10k::icons::icon;
use crate::extensions::p10k::render::Segment;
use crate::ported::params::{getsparam, setsparam, unsetparam};
use crate::ported::utils::getshfunc;

// ---------------------------------------------------------------------------
// Shared plumbing
// ---------------------------------------------------------------------------

/// Non-empty parameter/environment read. p10k segments test `$VAR`
/// (empty and unset are both "absent"), so both collapse to None.
fn envv(name: &str) -> Option<String> {
    getsparam(name)
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var(name).ok().filter(|v| !v.is_empty()))
}

/// p10k:203-210 — `_p9k_fetch_cwd`: logical $PWD when absolute, else
/// the process cwd.
fn cwd() -> PathBuf {
    if let Some(pwd) = envv("PWD") {
        let p = PathBuf::from(&pwd);
        if p.is_absolute() {
            return p;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

fn home() -> PathBuf {
    envv("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// mtime in epoch seconds; -1 when the path can't be stat'd — mirrors
/// `zstat -A stat +mtime -- $1 2>/dev/null || stat=(-1)` (p10k:187-188).
fn mtime(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(-1)
}

type StatSig = Vec<(PathBuf, i64)>;

/// Stat-keyed value cache — port of `_p9k_cache_stat_get` /
/// `_p9k_cache_stat_set`: a cached value stays valid while every
/// witness file keeps its mtime (missing file == mtime -1, also a
/// valid, comparable state).
static STAT_CACHE: OnceLock<Mutex<HashMap<String, (StatSig, Vec<String>)>>> = OnceLock::new();

fn stat_sig(files: &[PathBuf]) -> StatSig {
    files.iter().map(|f| (f.clone(), mtime(f))).collect()
}

fn cache_get(key: &str, files: &[PathBuf]) -> Option<Vec<String>> {
    let sig = stat_sig(files);
    let m = STAT_CACHE.get_or_init(Default::default);
    if let Ok(guard) = m.lock() {
        if let Some((cached_sig, vals)) = guard.get(key) {
            if *cached_sig == sig {
                return Some(vals.clone());
            }
        }
    }
    None
}

fn cache_set(key: &str, files: &[PathBuf], vals: Vec<String>) -> Vec<String> {
    let sig = stat_sig(files);
    let m = STAT_CACHE.get_or_init(Default::default);
    if let Ok(mut guard) = m.lock() {
        guard.insert(key.to_string(), (sig, vals.clone()));
    }
    vals
}

/// `$commands[name]` — locate an executable on $PATH. Cached against
/// the current $PATH string (zsh's `commands` hash rebuilds on PATH
/// change; this mirrors that invalidation).
fn have_cmd(name: &str) -> Option<PathBuf> {
    static CMD_CACHE: OnceLock<Mutex<HashMap<String, (String, Option<PathBuf>)>>> =
        OnceLock::new();
    let path_var = envv("PATH").unwrap_or_default();
    let m = CMD_CACHE.get_or_init(Default::default);
    if let Ok(guard) = m.lock() {
        if let Some((p, hit)) = guard.get(name) {
            if *p == path_var {
                return hit.clone();
            }
        }
    }
    let mut found = None;
    for dir in path_var.split(':').filter(|d| !d.is_empty()) {
        let cand = Path::new(dir).join(name);
        let is_exec = fs::metadata(&cand)
            .map(|md| {
                use std::os::unix::fs::PermissionsExt;
                md.is_file() && md.permissions().mode() & 0o111 != 0
            })
            .unwrap_or(false);
        if is_exec {
            found = Some(cand);
            break;
        }
    }
    if let Ok(mut guard) = m.lock() {
        guard.insert(name.to_string(), (path_var, found.clone()));
    }
    found
}

/// zsh `${commands[x]:-${${+functions[x]}:#0}}` — the init condition
/// most *env segments use: a command OR a shell function of that name.
fn cmd_or_func(name: &str) -> bool {
    have_cmd(name).is_some() || getshfunc(name).is_some()
}

/// Run a command, trimmed output. `extra_env` supports the
/// STACK_YAML=… prefix haskell_stack needs (p10k:5578).
fn run_cmd(
    bin: &Path,
    args: &[&str],
    merge_stderr: bool,
    extra_env: &[(&str, &str)],
) -> Option<(bool, String)> {
    let mut c = Command::new(bin);
    c.args(args);
    for (k, v) in extra_env {
        c.env(k, v);
    }
    // stdin MUST be nulled: a version tool inheriting the shell's RAW
    // tty stdin steals pending keystrokes from the ZLE input queue
    // (typed Enters vanished whenever a render ran these) and a tool
    // that decides to prompt interactively blocks the whole render.
    c.stdin(std::process::Stdio::null());
    if !merge_stderr {
        c.stderr(std::process::Stdio::null());
    }
    match c.output() {
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            if merge_stderr {
                text.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            Some((out.status.success(), text.trim_end_matches('\n').to_string()))
        }
        Err(e) => {
            tracing::debug!(target: "p10k", cmd = %bin.display(), %e, "command failed to spawn");
            None
        }
    }
}

/// Port of `_p9k_cached_cmd` (p10k:2416-2431): run `$cmd args...`,
/// cache (ok, output) keyed on the command line and invalidated by the
/// mtimes of the binary and an optional witness file ($2 in zsh).
/// `merge_stderr` mirrors `$1` (1 → `2>&1`, 0 → `2>/dev/null`).
fn cached_cmd(
    merge_stderr: bool,
    witness: Option<&Path>,
    cmd: &str,
    args: &[&str],
) -> Option<String> {
    // p10k:2417 — `local cmd=$commands[$3]; [[ -n $cmd ]] || return`
    let bin = have_cmd(cmd)?;
    let key = format!(
        "cached_cmd {} {} {}",
        cmd,
        args.join(" "),
        witness.map(|w| w.display().to_string()).unwrap_or_default()
    );
    let mut files: Vec<PathBuf> = Vec::new();
    if let Some(w) = witness {
        files.push(w.to_path_buf());
    }
    files.push(bin.clone());
    let vals = match cache_get(&key, &files) {
        Some(v) => v,
        None => {
            // p10k:2419-2426 — run once, remember (exit-ok, output)
            let (ok, text) = run_cmd(&bin, args, merge_stderr, &[]).unwrap_or((false, String::new()));
            cache_set(
                &key,
                &files,
                vec![if ok { "1".into() } else { "0".into() }, text],
            )
        }
    };
    // p10k:2429-2430 — only a successful run yields a value
    if vals.first().map(String::as_str) == Some("1") {
        Some(vals.get(1).cloned().unwrap_or_default())
    } else {
        None
    }
}

/// Port of `_p9k_read_word` (p10k:186-201): first whitespace-delimited
/// word of the first line of a file, CR-stripped; None when empty or
/// unreadable. The zsh version is mtime-cached; a single small read
/// per prompt is cheap enough to skip that layer.
fn read_word(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let first_line = content.lines().next()?.trim_end_matches('\r');
    let word = first_line.split_whitespace().next()?.to_string();
    if word.is_empty() { None } else { Some(word) }
}

/// Port of `_p9k_read_pyenv_like_version_file` (p10k:4248-4266): first
/// word of every line (first 1024 bytes), comment lines dropped, the
/// given prefix stripped, results joined with `:`.
fn read_pyenv_like_version_file(path: &Path, prefix: &str) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let slice = &bytes[..bytes.len().min(1024)]; // p10k:4257 — sysread -s 1024
    let content = String::from_utf8_lossy(slice);
    let mut versions: Vec<String> = Vec::new();
    for line in content.lines() {
        // p10k:4261 — `${MATCH[(w)1]}` first word, `##\#*` drops comments
        let Some(word) = line.split_whitespace().next() else { continue };
        if word.starts_with('#') {
            continue;
        }
        versions.push(word.strip_prefix(prefix).unwrap_or(word).to_string());
    }
    let joined = versions.join(":"); // p10k:4262 — `${(j.:.)versions}`
    if joined.is_empty() { None } else { Some(joined) }
}

/// Port of `_p9k_upglob` (p10k:265-292) for a literal file name: walk
/// from $PWD to `/`, return the first ancestor directory containing
/// `name`. `None` == zsh return 0 == "not found".
fn upfind(name: &str) -> Option<PathBuf> {
    upfind_pred(&|dir: &Path| dir.join(name).exists())
}

/// Predicate flavor for the multi-pattern upglobs
/// (`pom.xml|build.gradle.kts|...|*.(java|class|jar|...)`).
fn upfind_pred(pred: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let start = cwd();
    let mut dir: &Path = &start;
    loop {
        if pred(dir) {
            return Some(dir.to_path_buf());
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return None,
        }
    }
}

/// True when any entry of `dir` has one of `exts` as its extension —
/// the `*.(java|class|...)` half of glob-style upglobs.
fn dir_has_ext(dir: &Path, exts: &[&str]) -> bool {
    let Ok(rd) = fs::read_dir(dir) else { return false };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(ext) = name.rsplit('.').next() {
            if name.contains('.') && exts.contains(&ext) {
                return true;
            }
        }
    }
    false
}

/// Prompt-escape visible content — p10k's ubiquitous `${v//\%/%%}`.
fn esc(s: &str) -> String {
    s.replace('%', "%%")
}

/// p10k:8390-8396 — `_p9k_color1`: 7 under COLOR_SCHEME=light, else 0.
fn color1() -> String {
    if p9k_global("COLOR_SCHEME", "") == "light" { "7".into() } else { "0".into() }
}

/// Segment constructor. Arguments keep p10k's `_p9k_prompt_segment`
/// order — (background, foreground) — so call sites can be checked
/// against the zsh source verbatim (p10k:604-614).
fn seg(
    name: &str,
    state: Option<&str>,
    bg: &str,
    fg: &str,
    icon_key: &str,
    content: String,
) -> Segment {
    let glyph = if icon_key.is_empty() {
        None
    } else {
        let g = icon(icon_key);
        if g.is_empty() { None } else { Some(g.to_string()) }
    };
    Segment {
        name: name.to_string(),
        state: state.map(|s| s.to_ascii_uppercase()),
        content,
        icon: glyph,
        fg: fg.to_string(),
        bg: bg.to_string(),
    }
}

/// "Handled, hidden" — the zsh `return` before `_p9k_prompt_segment`.
fn hidden() -> Option<Vec<Segment>> {
    Some(Vec::new())
}

fn one(s: Segment) -> Option<Vec<Segment>> {
    Some(vec![s])
}

/// Minimal zsh-glob matcher for `*`-only patterns — enough for the
/// _CLASSES tables in real configs (`'*prod*' PROD`, `'*' DEFAULT`).
/// Other zsh glob operators are not supported; a pattern using them
/// simply won't match.
fn glob_match(pat: &str, text: &str) -> bool {
    fn inner(p: &[u8], t: &[u8]) -> bool {
        if p.is_empty() {
            return t.is_empty();
        }
        if p[0] == b'*' {
            (0..=t.len()).any(|i| inner(&p[1..], &t[i..]))
        } else {
            !t.is_empty() && p[0] == t[0] && inner(&p[1..], &t[1..])
        }
    }
    inner(pat.as_bytes(), text.as_bytes())
}

/// Port of the `for pat class in $_POWERLEVEL9K_<SEG>_CLASSES` loop
/// (e.g. p10k:1184-1189 for aws): first matching pattern picks the
/// class; the class (uppercased) becomes the segment state.
fn classes_state(segment: &str, text: &str) -> Option<String> {
    let classes = p9k_param_arr(segment, None, "CLASSES");
    let mut it = classes.iter();
    while let (Some(pat), Some(class)) = (it.next(), it.next()) {
        if glob_match(pat, text) {
            if class.is_empty() {
                return None;
            }
            // p10k:1186 — `state=_${${(U)class}//İ/I}`
            return Some(class.to_ascii_uppercase());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Generic <tool>env template (rbenv, nodenv, luaenv, jenv, plenv,
// phpenv, scalaenv share prompt_rbenv's structure verbatim;
// pyenv/goenv use the pyenv-like multi-version reader)
// ---------------------------------------------------------------------------

struct EnvTool {
    /// segment name, e.g. "rbenv" (also the command/function gate)
    seg: &'static str,
    /// env var prefix, e.g. "RBENV" (VERSION / DIR / ROOT vars)
    prefix: &'static str,
    /// per-directory version file, e.g. ".ruby-version"
    version_file: &'static str,
    /// default root under $HOME, e.g. ".rbenv"
    root_default: &'static str,
    /// strip prefix for pyenv-like multi-version files ("" == use
    /// `_p9k_read_word` single-word semantics)
    file_prefix: &'static str,
}

impl EnvTool {
    fn root(&self) -> PathBuf {
        envv(&format!("{}_ROOT", self.prefix))
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(self.root_default))
    }

    fn read_version_file(&self, path: &Path) -> Option<String> {
        if self.file_prefix.is_empty() {
            read_word(path)
        } else {
            read_pyenv_like_version_file(path, self.file_prefix)
        }
    }

    /// `_p9k_rbenv_global_version` et al (p10k:2753-2755, 4267-4269,
    /// 4351-4353, ...): `<root>/version` else "system".
    fn global_version(&self) -> String {
        self.read_version_file(&self.root().join("version"))
            .unwrap_or_else(|| "system".to_string())
    }

    fn sources(&self) -> Vec<String> {
        let s = p9k_param_arr(self.seg, None, "SOURCES");
        if s.is_empty() {
            // p10k:7423 etc — `_p9k_declare -a ..._SOURCES -- shell local global`
            vec!["shell".into(), "local".into(), "global".into()]
        } else {
            s
        }
    }

    /// The shared body of prompt_rbenv (p10k:2760-2812) — returns the
    /// version to display, or None when the segment hides.
    fn resolve(&self) -> Option<String> {
        let sources = self.sources();
        let has = |what: &str| sources.iter().any(|s| s == what);

        // p10k:2761-2763 — shell source: $RBENV_VERSION
        let v = if let Some(shell_v) = envv(&format!("{}_VERSION", self.prefix)) {
            if !has("shell") {
                return None;
            }
            // pyenv/goenv strip the file prefix from each :-part
            // (p10k:4275 — `${(j.:.)${(@)${(s.:.)PYENV_VERSION}#python-}}`)
            if self.file_prefix.is_empty() {
                shell_v
            } else {
                shell_v
                    .split(':')
                    .map(|p| p.strip_prefix(self.file_prefix).unwrap_or(p))
                    .collect::<Vec<_>>()
                    .join(":")
            }
        } else {
            // p10k:2765 — need local|global in sources
            if !has("local") && !has("global") {
                return None;
            }
            let mut found: Option<String> = None;

            // p10k:2767-2780 — $RBENV_DIR chain (searched before cwd)
            if let Some(d) = envv(&format!("{}_DIR", self.prefix)).filter(|d| d != ".") {
                let dir = if Path::new(&d).is_absolute() {
                    PathBuf::from(&d)
                } else {
                    cwd().join(&d) // p10k:2768 — `"$_p9k__cwd_a/$RBENV_DIR"`
                };
                let dir = dir.canonicalize().unwrap_or(dir);
                let cwd_a = cwd().canonicalize().unwrap_or_else(|_| cwd());
                if dir != cwd_a {
                    let mut cur: &Path = &dir;
                    loop {
                        if let Some(v) = self.read_version_file(&cur.join(self.version_file)) {
                            // p10k:2772-2773 — a hit here requires "local"
                            if !has("local") {
                                return None;
                            }
                            found = Some(v);
                            break;
                        }
                        match cur.parent() {
                            Some(p) => cur = p,
                            None => break,
                        }
                    }
                }
            }

            // p10k:2782-2789 — upward search from cwd
            if found.is_none() {
                if let Some(dir) = upfind(self.version_file) {
                    if let Some(v) = self.read_version_file(&dir.join(self.version_file)) {
                        if !has("local") {
                            return None;
                        }
                        found = Some(v);
                    }
                }
            }

            // p10k:2790-2794 — global fallback, gated on ALWAYS_SHOW
            match found {
                Some(v) => v,
                None => {
                    if !p9k_param_bool(self.seg, None, "PROMPT_ALWAYS_SHOW", false) {
                        return None;
                    }
                    if !has("global") {
                        return None;
                    }
                    self.global_version()
                }
            }
        };

        // p10k:2798-2801 — hide when equal to the global version
        if !p9k_param_bool(self.seg, None, "PROMPT_ALWAYS_SHOW", false)
            && v == self.global_version()
        {
            return None;
        }
        // p10k:2803-2805 — hide "system" unless SHOW_SYSTEM (default true)
        if !p9k_param_bool(self.seg, None, "SHOW_SYSTEM", true) && v == "system" {
            return None;
        }
        Some(v)
    }
}

/// One *env segment: command/function gate, template resolve, render.
fn envtool_segment(tool: &EnvTool, bg: &str, fg: &str, icon_key: &str) -> Option<Vec<Segment>> {
    // init cond: `${commands[rbenv]:-${${+functions[rbenv]}:#0}}`
    // (p10k:2810-2812). Root-dir existence is accepted too: a tool
    // driven purely by shell init scripts still owns its root.
    if !cmd_or_func(tool.seg) && !tool.root().is_dir() {
        return hidden();
    }
    match tool.resolve() {
        Some(v) => one(seg(tool.seg, None, bg, fg, icon_key, esc(&v))),
        None => hidden(),
    }
}

// ---------------------------------------------------------------------------
// pyenv (needs the extra P9K_PYENV_PYTHON_VERSION resolution)
// ---------------------------------------------------------------------------

const PYENV_TOOL: EnvTool = EnvTool {
    seg: "pyenv",
    prefix: "PYENV",
    version_file: ".python-version",
    root_default: ".pyenv",
    file_prefix: "python-",
};

/// Port of `_p9k_pyenv_compute` (p10k:4271-4337). Returns the display
/// version and the resolved interpreter version
/// (P9K_PYENV_PYTHON_VERSION); None == segment hides. `force_show`
/// reproduces `_p9k_python_version`'s local ALWAYS_SHOW=1
/// SHOW_SYSTEM=1 SOURCES=(shell local global) override
/// (p10k:1109-1111).
fn pyenv_compute(force_show: bool) -> Option<(String, Option<String>)> {
    let tool = PYENV_TOOL;
    let v = if force_show {
        // Same resolution order with the two hide-gates forced open.
        if let Some(shell_v) = envv("PYENV_VERSION") {
            shell_v
                .split(':')
                .map(|p| p.strip_prefix("python-").unwrap_or(p))
                .collect::<Vec<_>>()
                .join(":")
        } else if let Some(dir) = upfind(".python-version") {
            read_pyenv_like_version_file(&dir.join(".python-version"), "python-")?
        } else {
            tool.global_version()
        }
    } else {
        tool.resolve()?
    };

    // p10k:4321-4334 — resolve each :-part against
    // ${PYENV_ROOT:-~/.pyenv}/versions/<name> (symlinks followed);
    // the first that lands inside the versions dir names the
    // interpreter version.
    let versions_dir = tool.root().join("versions");
    let versions_real = versions_dir.canonicalize().unwrap_or_else(|_| versions_dir.clone());
    let mut python_version: Option<String> = None;
    for name in v.split(':') {
        let cand = versions_dir.join(name);
        let real = cand.canonicalize().unwrap_or(cand);
        if let Ok(rest) = real.strip_prefix(&versions_real) {
            if let Some(first) = rest.components().next() {
                python_version = Some(first.as_os_str().to_string_lossy().into_owned());
                break;
            }
        }
    }
    Some((v, python_version))
}

/// p10k:4340-4344 — prompt_pyenv.
fn segment_pyenv() -> Option<Vec<Segment>> {
    // init cond p10k:4347 — `${commands[pyenv]:-${${+functions[pyenv]}:#0}}`
    if !cmd_or_func("pyenv") && !PYENV_TOOL.root().is_dir() {
        return hidden();
    }
    match pyenv_compute(false) {
        Some((v, py)) => {
            // p10k:4328-4331 — `typeset -g P9K_PYENV_PYTHON_VERSION`
            // (the user's PYENV_CONTENT_EXPANSION references it)
            match &py {
                Some(pv) => {
                    let _ = setsparam("P9K_PYENV_PYTHON_VERSION", pv);
                }
                None => {
                    let _ = unsetparam("P9K_PYENV_PYTHON_VERSION");
                }
            }
            // p10k:4342 — blue bg, color1 fg, PYTHON_ICON
            one(seg("pyenv", None, "blue", &color1(), "PYTHON_ICON", esc(&v)))
        }
        None => {
            let _ = unsetparam("P9K_PYENV_PYTHON_VERSION"); // p10k:4272
            hidden()
        }
    }
}

/// Port of `_p9k_python_version` (p10k:1104-1123): pyenv-shim aware
/// `python --version`.
fn python_version() -> Option<String> {
    let python = have_cmd("python")?; // p10k:1106 — no python, no version
    let shim = PYENV_TOOL.root().join("shims/python");
    if python == shim {
        // p10k:1108-1114 — force-shown pyenv result when it looks like
        // a version number
        if let Some((_, Some(pv))) = pyenv_compute(true) {
            if pv.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return Some(pv);
            }
        }
    }
    // p10k:1116-1118 — `python --version` (stderr merged: python2
    // prints the version to stderr), "Python X.Y.Z" → "X.Y.Z"
    let out = cached_cmd(true, None, "python", &["--version"])?;
    let rest = out.strip_prefix("Python ")?;
    let ver: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if ver.is_empty() { None } else { Some(ver) }
}

// ---------------------------------------------------------------------------
// Individual segments
// ---------------------------------------------------------------------------

/// p10k:4983-4987 — prompt_direnv: icon-only segment, shown iff
/// $DIRENV_DIR is set (the runtime cond `'$DIRENV_DIR'`).
fn segment_direnv() -> Option<Vec<Segment>> {
    if envv("DIRENV_DIR").is_none() {
        return hidden();
    }
    // p10k:4985 — `_p9k_prompt_segment $0 $_p9k_color1 yellow DIRENV_ICON 0 '$DIRENV_DIR' ''`
    one(seg("direnv", None, &color1(), "yellow", "DIRENV_ICON", String::new()))
}

/// p10k:4221-4241 — prompt_virtualenv.
fn segment_virtualenv() -> Option<Vec<Segment>> {
    // init cond p10k:4244 — `'$VIRTUAL_ENV'`
    let Some(venv) = envv("VIRTUAL_ENV") else { return hidden() };

    let mut msg = String::new();
    // p10k:4223-4225 — optional interpreter version prefix
    // (default 1, p10k:7507; the user's config sets false)
    if p9k_param_bool("virtualenv", None, "SHOW_PYTHON_VERSION", true) {
        if let Some(ver) = python_version() {
            msg.push_str(&esc(&ver));
            msg.push(' ');
        }
    }
    // p10k:4226-4227 — generic env dir names display the parent dir
    let vpath = PathBuf::from(&venv);
    let base = vpath
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut generics = p9k_param_arr("virtualenv", None, "GENERIC_NAMES");
    if generics.is_empty() {
        // p10k:7510 — default: virtualenv venv .venv env
        generics = vec!["virtualenv".into(), "venv".into(), ".venv".into(), "env".into()];
    }
    let v = if generics.iter().any(|g| glob_match(g, &base)) {
        vpath
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(base)
    } else {
        base
    };
    // p10k:4228 — LEFT_DELIMITER + name + RIGHT_DELIMITER
    // (defaults "(" ")", p10k:7508-7509)
    let l = p9k_param("virtualenv", None, "LEFT_DELIMITER", "(");
    let r = p9k_param("virtualenv", None, "RIGHT_DELIMITER", ")");
    msg.push_str(&format!("{l}{}{r}", esc(&v)));

    // p10k:4229-4239 — SHOW_WITH_PYENV interplay (default "true",
    // p10k:7506)
    match p9k_param("virtualenv", None, "SHOW_WITH_PYENV", "true").as_str() {
        "false" => {
            // p10k:4231 — cond `${(M)${#P9K_PYENV_PYTHON_VERSION}:#0}`:
            // hidden while pyenv resolves an interpreter version
            if let Some((_, Some(_))) = pyenv_compute(false) {
                return hidden();
            }
        }
        "if-different" => {
            // p10k:4233-4235 — hidden when equal to the pyenv version
            if let Some((pv, _)) = pyenv_compute(false) {
                if pv == v {
                    return hidden();
                }
            }
        }
        _ => {}
    }
    // p10k:4231/4235/4238 — blue bg, color1 fg, PYTHON_ICON
    one(seg("virtualenv", None, "blue", &color1(), "PYTHON_ICON", msg))
}

/// p10k:1131-1144 — prompt_anaconda.
fn segment_anaconda() -> Option<Vec<Segment>> {
    // init cond p10k:1147 — `'${CONDA_PREFIX:-$CONDA_ENV_PATH}'`
    let Some(p) = envv("CONDA_PREFIX").or_else(|| envv("CONDA_ENV_PATH")) else {
        let _ = unsetparam("P9K_ANACONDA_PYTHON_VERSION");
        return hidden();
    };
    let mut msg = String::new();
    // p10k:1133-1139 — python version prefix + P9K_ANACONDA_PYTHON_VERSION
    match python_version() {
        Some(ver) => {
            let _ = setsparam("P9K_ANACONDA_PYTHON_VERSION", &ver);
            // default 1, p10k:7260
            if p9k_param_bool("anaconda", None, "SHOW_PYTHON_VERSION", true) {
                msg.push_str(&esc(&ver));
                msg.push(' ');
            }
        }
        None => {
            let _ = unsetparam("P9K_ANACONDA_PYTHON_VERSION"); // p10k:1139
        }
    }
    // p10k:1141-1142 — delimiters around the env basename
    let base = Path::new(&p)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let l = p9k_param("anaconda", None, "LEFT_DELIMITER", "(");  // p10k:7258
    let r = p9k_param("anaconda", None, "RIGHT_DELIMITER", ")"); // p10k:7259
    msg.push_str(&format!("{l}{}{r}", esc(&base)));
    // p10k:1143 — blue bg, color1 fg, PYTHON_ICON
    one(seg("anaconda", None, "blue", &color1(), "PYTHON_ICON", msg))
}

/// p10k:2434-2445 — prompt_node_version.
fn segment_node_version() -> Option<Vec<Segment>> {
    if have_cmd("node").is_none() {
        return hidden(); // init cond p10k:2448 — `'$commands[node]'`
    }
    // p10k:2435-2442 — witness the nearest package.json; without one,
    // PROJECT_ONLY (default 0, p10k:7413; the user's config: true)
    // hides the segment
    let out = match upfind("package.json") {
        Some(dir) => cached_cmd(false, Some(&dir.join("package.json")), "node", &["--version"]),
        None => {
            if p9k_param_bool("node_version", None, "PROJECT_ONLY", false) {
                return hidden();
            }
            cached_cmd(false, None, "node", &["--version"])
        }
    };
    let Some(out) = out else { return hidden() };
    // p10k:2443 — `[[ $_p9k__ret == v?* ]] || return`
    let Some(v) = out.strip_prefix('v').filter(|r| !r.is_empty()) else {
        return hidden();
    };
    // p10k:2444 — green bg, white fg, NODE_ICON
    one(seg("node_version", None, "green", "white", "NODE_ICON", esc(v)))
}

/// p10k:2185-2202 — prompt_go_version.
fn segment_go_version() -> Option<Vec<Segment>> {
    if have_cmd("go").is_none() {
        return hidden(); // init cond p10k:2205 — `'$commands[go]'`
    }
    // p10k:2186 — `_p9k_cached_cmd 0 '' go version`
    let Some(out) = cached_cmd(false, None, "go", &["version"]) else {
        return hidden();
    };
    // p10k:2187 — `[[ $_p9k__ret == (#b)*go([[:digit:].]##)* ]]`
    let v = out.split_whitespace().find_map(|tok| {
        let rest = tok.strip_prefix("go")?;
        (!rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.'))
            .then(|| rest.to_string())
    });
    let Some(v) = v else { return hidden() };
    // p10k:2189-2200 — PROJECT_ONLY (default 1, p10k:7416): inside
    // GOPATH or under a go.mod
    if p9k_param_bool("go_version", None, "PROJECT_ONLY", true) {
        // p10k:2190-2196 — GOPATH: $GOPATH, else ~/go if it exists,
        // else `go env GOPATH`
        let gopath = envv("GOPATH").or_else(|| {
            let hg = home().join("go");
            if hg.is_dir() {
                Some(hg.to_string_lossy().into_owned())
            } else {
                cached_cmd(false, None, "go", &["env", "GOPATH"]).filter(|p| !p.is_empty())
            }
        });
        let in_gopath = gopath
            .map(|p| cwd().starts_with(Path::new(&p)))
            .unwrap_or(false);
        if !in_gopath && upfind("go.mod").is_none() {
            return hidden(); // p10k:2198 — `_p9k_upglob go.mod && return`
        }
    }
    // p10k:2201 — green bg, grey93 fg, GO_ICON
    one(seg("go_version", None, "green", "grey93", "GO_ICON", esc(&v)))
}

/// p10k:3144-3196 — prompt_rust_version.
fn segment_rust_version() -> Option<Vec<Segment>> {
    let Some(rustc) = have_cmd("rustc") else {
        let _ = unsetparam("P9K_RUST_VERSION"); // p10k:3145
        return hidden(); // init cond p10k:3199 — `'$commands[rustc]'`
    };
    // p10k:3146-3148 — PROJECT_ONLY (default 1, p10k:7417) gates on an
    // enclosing Cargo.toml
    if p9k_param_bool("rust_version", None, "PROJECT_ONLY", true) && upfind("Cargo.toml").is_none()
    {
        let _ = unsetparam("P9K_RUST_VERSION");
        return hidden();
    }
    // p10k:3163-3187 — toolchain: $RUSTUP_TOOLCHAIN, else the nearest
    // rust-toolchain file. (`rustup override list` consultation —
    // p10k:3169-3179 — is not ported: it shells out per settings
    // change to enumerate overrides that RUSTUP_TOOLCHAIN or
    // rust-toolchain cover in typical setups; the toolchain only
    // feeds the cache key here.)
    let toolchain = envv("RUSTUP_TOOLCHAIN").or_else(|| {
        upfind("rust-toolchain").and_then(|d| read_word(&d.join("rust-toolchain")))
    });
    // p10k:3160-3162 — rustup settings are cache witnesses
    let rustup_home = envv("RUSTUP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".rustup"));
    let key = format!("rust_version {}", toolchain.as_deref().unwrap_or(""));
    let files = vec![rustc.clone(), rustup_home.join("settings.toml")];
    // p10k:3189-3191 — `$rustc --version`, stat-cached
    let vals = match cache_get(&key, &files) {
        Some(v) => v,
        None => {
            let out = run_cmd(&rustc, &["--version"], false, &[])
                .filter(|(ok, _)| *ok)
                .map(|(_, text)| text)
                .unwrap_or_default();
            cache_set(&key, &files, vec![out])
        }
    };
    let full = vals.first().cloned().unwrap_or_default();
    // p10k:3192 — `${${_p9k__cache_val[1]#rustc }%% *}`
    let v = full
        .strip_prefix("rustc ")
        .unwrap_or(&full)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if v.is_empty() {
        let _ = unsetparam("P9K_RUST_VERSION");
        return hidden(); // p10k:3193 — `[[ -n $v ]] || return`
    }
    let _ = setsparam("P9K_RUST_VERSION", &full); // p10k:3194
    // p10k:3195 — darkorange bg, color1 fg, RUST_ICON
    one(seg("rust_version", None, "darkorange", &color1(), "RUST_ICON", esc(&v)))
}

/// p10k:2663-2667 — prompt_os_icon: the per-OS glyph resolved at init
/// by `_p9k_set_os` (p10k:8340-8388).
fn segment_os_icon() -> Option<Vec<Segment>> {
    let key = if cfg!(target_os = "macos") {
        "APPLE_ICON" // p10k:8347 — `Darwin) _p9k_set_os OSX APPLE_ICON`
    } else if cfg!(target_os = "freebsd") || cfg!(target_os = "openbsd") {
        "FREEBSD_ICON" // p10k:8349
    } else if cfg!(target_os = "android") {
        "ANDROID_ICON" // p10k:8343
    } else if cfg!(target_os = "linux") {
        // p10k:8352-8386 — distro-specific icon from /etc/os-release ID
        linux_distro_icon()
    } else {
        "LINUX_ICON"
    };
    let glyph = icon(key);
    let glyph = if glyph.is_empty() { icon("LINUX_ICON") } else { glyph };
    // p10k:2665 — `_p9k_prompt_segment "$0" "black" "white" '' 0 '' "$_p9k_os_icon"`
    // (the glyph IS the content; no separate visual identifier)
    one(seg("os_icon", None, "black", "white", "", glyph.to_string()))
}

/// p10k:8352-8386 — map /etc/os-release ID to a LINUX_*_ICON key.
fn linux_distro_icon() -> &'static str {
    let content = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let id = content
        .lines()
        .find_map(|l| l.strip_prefix("ID="))
        .unwrap_or("")
        .trim_matches('"')
        .to_ascii_lowercase();
    if id == "amzn" {
        return "LINUX_AMZN_ICON"; // p10k:8383 — exact match, no glob
    }
    // p10k:8361-8384 — `*arch*` etc: substring matches
    const MAP: &[(&str, &str)] = &[
        ("arch", "LINUX_ARCH_ICON"),
        ("debian", "LINUX_DEBIAN_ICON"),
        ("raspbian", "LINUX_RASPBIAN_ICON"),
        ("ubuntu", "LINUX_UBUNTU_ICON"),
        ("elementary", "LINUX_ELEMENTARY_ICON"),
        ("fedora", "LINUX_FEDORA_ICON"),
        ("coreos", "LINUX_COREOS_ICON"),
        ("kali", "LINUX_KALI_ICON"),
        ("gentoo", "LINUX_GENTOO_ICON"),
        ("mageia", "LINUX_MAGEIA_ICON"),
        ("centos", "LINUX_CENTOS_ICON"),
        ("opensuse", "LINUX_OPENSUSE_ICON"),
        ("tumbleweed", "LINUX_OPENSUSE_ICON"),
        ("sabayon", "LINUX_SABAYON_ICON"),
        ("slackware", "LINUX_SLACKWARE_ICON"),
        ("linuxmint", "LINUX_MINT_ICON"),
        ("alpine", "LINUX_ALPINE_ICON"),
        ("aosc", "LINUX_AOSC_ICON"),
        ("nixos", "LINUX_NIXOS_ICON"),
        ("devuan", "LINUX_DEVUAN_ICON"),
        ("manjaro", "LINUX_MANJARO_ICON"),
        ("void", "LINUX_VOID_ICON"),
        ("artix", "LINUX_ARTIX_ICON"),
        ("rhel", "LINUX_RHEL_ICON"),
    ];
    for (pat, key) in MAP {
        if id.contains(pat) {
            return key;
        }
    }
    "LINUX_ICON" // p10k:8384 — `*)`
}

/// p10k:4560-4576 — prompt_java_version.
fn segment_java_version() -> Option<Vec<Segment>> {
    let Some(java) = have_cmd("java") else {
        return hidden(); // init cond p10k:4579 — `'$commands[java]'`
    };
    // p10k:4561-4563 — PROJECT_ONLY (default 0, p10k:7418; the user's
    // config: true): a java project marker up the tree
    if p9k_param_bool("java_version", None, "PROJECT_ONLY", false) {
        const NAMES: &[&str] = &[
            "pom.xml", "build.gradle.kts", "build.sbt", "deps.edn", "project.clj", "build.boot",
        ];
        const EXTS: &[&str] = &["java", "class", "jar", "gradle", "clj", "cljc"];
        let found = upfind_pred(&|dir: &Path| {
            NAMES.iter().any(|n| dir.join(n).exists()) || dir_has_ext(dir, EXTS)
        });
        if found.is_none() {
            return hidden();
        }
    }
    // p10k:4565-4572 — `java -fullversion 2>&1` cached against the java
    // binary and $JAVA_HOME/release; version is the quoted token
    let mut files = vec![java.clone()];
    if let Some(jh) = envv("JAVA_HOME") {
        files.push(Path::new(&jh).join("release"));
    }
    let vals = match cache_get("java_version", &files) {
        Some(v) => v,
        None => {
            let raw = run_cmd(&java, &["-fullversion"], true, &[])
                .map(|(_, text)| text)
                .unwrap_or_default();
            // p10k:4568 — `v=${${v#*\"}%\"*}`
            let v = raw
                .split_once('"')
                .and_then(|(_, rest)| rest.rsplit_once('"'))
                .map(|(v, _)| v.to_string())
                .unwrap_or_default();
            // p10k:4569 — short form (FULL default 1, p10k:7552; the
            // user's config: false) truncates at the first '-'
            let v = if p9k_param_bool("java_version", None, "FULL", true) {
                v
            } else {
                v.split('-').next().unwrap_or("").to_string()
            };
            cache_set("java_version", &files, vec![esc(&v)]) // p10k:4570
        }
    };
    let v = vals.first().cloned().unwrap_or_default();
    if v.is_empty() {
        return hidden(); // p10k:4574
    }
    // p10k:4575 — red bg, white fg, JAVA_ICON
    one(seg("java_version", None, "red", "white", "JAVA_ICON", v))
}

/// p10k:2217-2268 — prompt_package: name@version from the nearest
/// package.json. p10k hand-rolls a JSON scanner ("Redneck json
/// parsing. Yields correct results for any well-formed json
/// document." — p10k:2226-2227) to avoid a jq dependency; serde_json
/// is exact under the same well-formed-document contract.
fn segment_package() -> Option<Vec<Segment>> {
    let _ = unsetparam("P9K_PACKAGE_NAME"); // p10k:2218
    let _ = unsetparam("P9K_PACKAGE_VERSION");
    // p10k:2219 — `_p9k_upglob package.json && return`
    let Some(dir) = upfind("package.json") else { return hidden() };
    let file = dir.join("package.json");
    let key = format!("package {}", file.display());
    let files = vec![file.clone()];
    let vals = match cache_get(&key, &files) {
        Some(v) => v,
        None => {
            let parsed: Option<(String, String)> = fs::read_to_string(&file)
                .ok()
                .and_then(|data| {
                    serde_json::from_str::<serde_json::Value>(data.trim_start_matches('\u{feff}'))
                        .ok()
                })
                .and_then(|json| {
                    let name = json.get("name")?.as_str()?.to_string();
                    let version = json.get("version")?.as_str()?.to_string();
                    // p10k:2251-2253 — reject empty / newline / backslash
                    let bad = |s: &str| s.is_empty() || s.contains('\n') || s.contains('\\');
                    if bad(&name) || bad(&version) {
                        None
                    } else {
                        Some((name, version))
                    }
                });
            match parsed {
                // p10k:2260 — `_p9k_cache_stat_set 1 $found[name] $found[version]`
                Some((n, v)) => cache_set(&key, &files, vec!["1".into(), n, v]),
                None => cache_set(&key, &files, vec!["0".into()]), // p10k:2262
            }
        }
    };
    if vals.first().map(String::as_str) != Some("1") {
        return hidden(); // p10k:2264 — `(( _p9k__cache_val[1] )) || return`
    }
    let name = vals.get(1).cloned().unwrap_or_default();
    let version = vals.get(2).cloned().unwrap_or_default();
    let _ = setsparam("P9K_PACKAGE_NAME", &name); // p10k:2266-2267
    let _ = setsparam("P9K_PACKAGE_VERSION", &version);
    // p10k:2268 — cyan bg, color1 fg, PACKAGE_ICON, version content
    one(seg("package", None, "cyan", &color1(), "PACKAGE_ICON", esc(&version)))
}

/// p10k:3221-3230 — prompt_rvm.
fn segment_rvm() -> Option<Vec<Segment>> {
    // p10k:3222 — `[[ $GEM_HOME == *rvm* && $ruby_string != $rvm_path/bin/ruby ]]`
    let Some(gem_home) = envv("GEM_HOME") else { return hidden() };
    if !gem_home.contains("rvm") {
        return hidden();
    }
    let ruby_string = getsparam("ruby_string").unwrap_or_default();
    let rvm_path = getsparam("rvm_path").unwrap_or_default();
    if ruby_string == format!("{rvm_path}/bin/ruby") {
        return hidden();
    }
    // p10k:3223 — `local v=${GEM_HOME:t}`
    let mut v = Path::new(&gem_home)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // p10k:3224 — strip @gemset unless SHOW_GEMSET (default 0,
    // p10k:7461; separator from $rvm_gemset_separator, default '@')
    if !p9k_param_bool("rvm", None, "SHOW_GEMSET", false) {
        let sep = getsparam("rvm_gemset_separator")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "@".into());
        if let Some(idx) = v.find(&sep) {
            v.truncate(idx);
        }
    }
    // p10k:3225 — `v=${v#*-}` strips the interpreter prefix (ruby-)
    // unless SHOW_PREFIX (default 0, p10k:7462)
    if !p9k_param_bool("rvm", None, "SHOW_PREFIX", false) {
        if let Some(idx) = v.find('-') {
            v = v[idx + 1..].to_string();
        }
    }
    if v.is_empty() {
        return hidden(); // p10k:3226
    }
    // p10k:3227 — bg 240, color1 fg, RUBY_ICON
    one(seg("rvm", None, "240", &color1(), "RUBY_ICON", esc(&v)))
}

/// p10k:1306-1332 — prompt_fvm (`_p9k_fvm_new || _p9k_fvm_old`):
/// flutter version from the .fvm/flutter_sdk (new) or fvm (old)
/// symlink target.
fn segment_fvm() -> Option<Vec<Segment>> {
    if have_cmd("fvm").is_none() {
        return hidden(); // init cond p10k:1335 — `'$commands[fvm]'`
    }
    // shared: symlink target `.../versions/<v>[/bin/flutter]` → <v>
    fn version_from_link(link: &Path, strip_suffix: &str) -> Option<String> {
        let target = fs::read_link(link).ok()?;
        let t = target.to_string_lossy();
        let t = t.strip_suffix(strip_suffix).unwrap_or(&t);
        let (head, v) = t.rsplit_once('/')?;
        (head.ends_with("versions") && !v.is_empty()).then(|| v.to_string())
    }
    // p10k:1318-1328 — _p9k_fvm_new: <dir>/.fvm/flutter_sdk → */versions/(X)
    if let Some(dir) = upfind(".fvm") {
        if let Some(v) = version_from_link(&dir.join(".fvm/flutter_sdk"), "") {
            // p10k:1323 — blue bg, color1 fg, FLUTTER_ICON
            return one(seg("fvm", None, "blue", &color1(), "FLUTTER_ICON", esc(&v)));
        }
    }
    // p10k:1306-1316 — _p9k_fvm_old: <dir>/fvm → */versions/(X)/bin/flutter
    if let Some(dir) = upfind("fvm") {
        if let Some(v) = version_from_link(&dir.join("fvm"), "/bin/flutter") {
            return one(seg("fvm", None, "blue", &color1(), "FLUTTER_ICON", esc(&v)));
        }
    }
    hidden()
}

/// p10k:2452-2528 — `_p9k_nvm_ls_default`: resolve the `default`
/// alias chain to a concrete version. None == "no usable default"
/// (zsh return 1), which makes prompt_nvm SHOW the current version.
fn nvm_ls_default(nvm_dir: &Path) -> Option<String> {
    // p10k:2453-2463 — follow alias files with loop detection
    let mut v = "default".to_string();
    let mut seen = vec![v.clone()];
    loop {
        let alias = nvm_dir.join("alias").join(&v);
        let Ok(content) = fs::read_to_string(&alias) else { break };
        // p10k:2457 — `IFS='' read -r target` (whole first line)
        let target = content
            .lines()
            .next()
            .unwrap_or("")
            .trim_end_matches('\r')
            .to_string();
        if target.is_empty() {
            break; // p10k:2460 — `[[ -z $target ]] && break`
        }
        if seen.contains(&target) {
            return None; // p10k:2461 — alias cycle
        }
        seen.push(target.clone());
        v = target;
    }
    // p10k:2465-2480 — normalize
    match v.as_str() {
        "default" | "N/A" => return None, // p10k:2466-2468
        "system" | "v" => return Some("system".to_string()), // p10k:2469-2472
        _ => {}
    }
    if let Some(rest) = v.strip_prefix("iojs-") {
        // p10k:2473-2475 — `iojs-[0-9]*` → `iojs-v...`
        if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            v = format!("iojs-v{rest}");
        }
    } else if v.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        v = format!("v{v}"); // p10k:2476-2478
    }

    // p10k:2482-2493 — exact vX.Y.Z: check install dirs
    if v.starts_with('v') && v.matches('.').count() >= 2 {
        if nvm_dir.join("versions/node").join(&v).join("bin/node").exists()
            || nvm_dir.join(&v).join("bin/node").exists()
        {
            return Some(v);
        }
        if nvm_dir.join("versions/io.js").join(&v).join("bin/node").exists() {
            return Some(format!("iojs-{v}"));
        }
        return None;
    }

    // p10k:2495-2527 — fuzzy: scan install dirs for the max matching
    // version. p10k pads each of the 3 numeric components to 6 digits
    // and compares strings; a numeric tuple compare has the same order.
    fn parse(name: &str) -> Option<(u64, u64, u64)> {
        let rest = name.strip_prefix('v')?;
        let mut it = rest.split('.');
        let a = it.next()?.parse().ok()?;
        let b = it.next()?.parse().ok()?;
        let c = it.next()?.parse().ok()?;
        Some((a, b, c))
    }
    let (dirs, accept): (Vec<PathBuf>, Box<dyn Fn(&str, (u64, u64, u64)) -> bool>) =
        match v.as_str() {
            // p10k:2497-2500 — stable: v1+ or even v0 minor
            "node" | "node-" | "stable" => (
                vec![nvm_dir.join("versions/node"), nvm_dir.to_path_buf()],
                Box::new(|_, (a, b, _)| a >= 1 || b % 2 == 0),
            ),
            // p10k:2501-2504 — unstable: odd v0 minor
            "unstable" => (
                vec![nvm_dir.join("versions/node"), nvm_dir.to_path_buf()],
                Box::new(|_, (a, b, _)| a == 0 && b % 2 == 1),
            ),
            // p10k:2505-2508 — iojs*: normalized version prefix
            _ if v.starts_with("iojs") => {
                let want = format!(
                    "v{}",
                    v.trim_start_matches("iojs")
                        .trim_start_matches('-')
                        .trim_start_matches('v')
                );
                (
                    vec![nvm_dir.join("versions/io.js")],
                    Box::new(move |name: &str, _| name.starts_with(&want)),
                )
            }
            // p10k:2509-2512 — any other prefix
            _ => {
                let want = format!("v{}", v.trim_start_matches('v'));
                (
                    vec![
                        nvm_dir.join("versions/node"),
                        nvm_dir.to_path_buf(),
                        nvm_dir.join("versions/io.js"),
                    ],
                    Box::new(move |name: &str, _| name.starts_with(&want)),
                )
            }
        };
    let mut best: Option<((u64, u64, u64), String)> = None;
    for d in &dirs {
        let Ok(rd) = fs::read_dir(d) else { continue };
        for entry in rd.flatten() {
            if !entry.path().is_dir() {
                continue; // p10k:2515 — `(/N)` glob qualifier: dirs only
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(nums) = parse(&name) else { continue }; // p10k:2520
            if !accept(&name, nums) {
                continue;
            }
            // p10k:2521-2526 — keep the max; io.js dirs display as iojs-v...
            let display = if d.ends_with("io.js") { format!("iojs-{name}") } else { name };
            if best.as_ref().map(|(n, _)| nums > *n).unwrap_or(true) {
                best = Some((nums, display));
            }
        }
    }
    best.map(|(_, name)| name)
}

/// p10k:2530-2560 — `_p9k_nvm_ls_current` + prompt_nvm.
fn segment_nvm() -> Option<Vec<Segment>> {
    // p10k:2551 — `[[ -n $NVM_DIR ]] && _p9k_nvm_ls_current || return`
    let Some(nvm_dir_s) = envv("NVM_DIR") else { return hidden() };
    let nvm_dir = PathBuf::from(&nvm_dir_s);

    // _p9k_nvm_ls_current (p10k:2530-2545)
    let Some(node) = have_cmd("node") else { return hidden() }; // p10k:2531-2532
    let node_real = node.canonicalize().unwrap_or(node);
    let nvm_real = nvm_dir.canonicalize().unwrap_or_else(|_| nvm_dir.clone());
    let current = if node_real.starts_with(nvm_real.join("versions/io.js")) {
        // p10k:2535-2537 — iojs under NVM_DIR
        let Some(v) = cached_cmd(false, None, "iojs", &["--version"]) else {
            return hidden();
        };
        format!("iojs-v{}", v.trim_start_matches('v'))
    } else if node_real.starts_with(&nvm_real) {
        // p10k:2538-2540 — node under NVM_DIR
        let Some(v) = cached_cmd(false, None, "node", &["--version"]) else {
            return hidden();
        };
        format!("v{}", v.trim_start_matches('v'))
    } else {
        "system".to_string() // p10k:2542
    };

    // p10k:2553 — `! _p9k_nvm_ls_default || [[ $_p9k__ret != $current ]] || return`
    if let Some(default) = nvm_ls_default(&nvm_dir) {
        if default == current {
            return hidden();
        }
    }
    // p10k:2554 — magenta bg, black fg, NODE_ICON, `${current#v}`
    let shown = current.strip_prefix('v').unwrap_or(&current);
    one(seg("nvm", None, "magenta", "black", "NODE_ICON", esc(shown)))
}

/// p10k:2563-2571 — prompt_nodeenv.
fn segment_nodeenv() -> Option<Vec<Segment>> {
    // init cond p10k:2574 — `'$NODE_VIRTUAL_ENV'`
    let Some(nve) = envv("NODE_VIRTUAL_ENV") else { return hidden() };
    let mut msg = String::new();
    // p10k:2565-2567 — optional `node --version` prefix (default 1,
    // p10k:7512)
    if p9k_param_bool("nodeenv", None, "SHOW_NODE_VERSION", true) {
        if let Some(v) = cached_cmd(false, None, "node", &["--version"]) {
            msg.push_str(&esc(&v));
            msg.push(' ');
        }
    }
    // p10k:2568 — delimiters around the env basename (defaults "[" "]",
    // p10k:7513-7514)
    let base = Path::new(&nve)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let l = p9k_param("nodeenv", None, "LEFT_DELIMITER", "[");
    let r = p9k_param("nodeenv", None, "RIGHT_DELIMITER", "]");
    msg.push_str(&format!("{l}{}{r}", esc(&base)));
    // p10k:2569 — black bg, green fg, NODE_ICON
    one(seg("nodeenv", None, "black", "green", "NODE_ICON", msg))
}

/// p10k:2577-2585 — `_p9k_nodeenv_version_transform`: canonicalize a
/// requested node version against installed
/// ${NODENV_ROOT:-~/.nodenv}/versions entries; None == hide.
fn nodenv_version_transform(v: &str) -> Option<String> {
    let dir = envv("NODENV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".nodenv"))
        .join("versions");
    if v.is_empty() || v == "system" {
        return Some(v.to_string()); // p10k:2579
    }
    let candidates = [
        v.to_string(),                                     // p10k:2580
        v.replacen('v', "", 1),                            // p10k:2581 — `${1/v}`
        v.strip_prefix("node-").unwrap_or(v).to_string(),  // p10k:2582
        v.strip_prefix("node-v").unwrap_or(v).to_string(), // p10k:2583
    ];
    for c in candidates {
        if dir.join(&c).is_dir() {
            return Some(c);
        }
    }
    None // p10k:2584 — `return 1`
}

/// p10k:2593-2643 — prompt_nodenv: the rbenv template plus the
/// version transform.
fn segment_nodenv() -> Option<Vec<Segment>> {
    const TOOL: EnvTool = EnvTool {
        seg: "nodenv",
        prefix: "NODENV",
        version_file: ".node-version",
        root_default: ".nodenv",
        file_prefix: "",
    };
    if !cmd_or_func("nodenv") && !TOOL.root().is_dir() {
        return hidden(); // init cond p10k:2646
    }
    let always_show = p9k_param_bool("nodenv", None, "PROMPT_ALWAYS_SHOW", false);
    // shell source keeps the raw value (p10k:2594-2596); file/global
    // sources run through the transform (p10k:2631)
    let from_shell = envv("NODENV_VERSION").is_some();
    let Some(v) = TOOL.resolve() else { return hidden() };
    let v = if from_shell {
        v
    } else {
        // p10k:2631 — `_p9k_nodeenv_version_transform $_p9k__ret || return`
        match nodenv_version_transform(&v) {
            Some(t) => t,
            None => return hidden(),
        }
    };
    // p10k:2635-2638 — the always-show compare transforms the global
    // version too (EnvTool::resolve compared untransformed; re-check)
    if !always_show {
        if let Some(g) = nodenv_version_transform(&TOOL.global_version()) {
            if g == v {
                return hidden();
            }
        }
    }
    // p10k:2640-2642 — system gate (resolve() already applied it, but
    // the transform may have produced "system")
    if !p9k_param_bool("nodenv", None, "SHOW_SYSTEM", true) && v == "system" {
        return hidden();
    }
    // p10k:2643 — black bg, green fg, NODE_ICON
    one(seg("nodenv", None, "black", "green", "NODE_ICON", esc(&v)))
}

/// p10k:4355-4404 — prompt_goenv (pyenv template, go- prefix).
fn segment_goenv() -> Option<Vec<Segment>> {
    const TOOL: EnvTool = EnvTool {
        seg: "goenv",
        prefix: "GOENV",
        version_file: ".go-version",
        root_default: ".goenv",
        file_prefix: "go-",
    };
    // p10k:4404 — blue bg, color1 fg, GO_ICON
    envtool_segment(&TOOL, "blue", &color1(), "GO_ICON")
}

/// p10k:2760-2812 — prompt_rbenv.
fn segment_rbenv() -> Option<Vec<Segment>> {
    const TOOL: EnvTool = EnvTool {
        seg: "rbenv",
        prefix: "RBENV",
        version_file: ".ruby-version",
        root_default: ".rbenv",
        file_prefix: "",
    };
    // p10k:2807 — red bg, color1 fg, RUBY_ICON
    envtool_segment(&TOOL, "red", &color1(), "RUBY_ICON")
}

/// p10k:2942-2996 — prompt_luaenv.
fn segment_luaenv() -> Option<Vec<Segment>> {
    const TOOL: EnvTool = EnvTool {
        seg: "luaenv",
        prefix: "LUAENV",
        version_file: ".lua-version",
        root_default: ".luaenv",
        file_prefix: "",
    };
    // p10k:2996 — blue bg, color1 fg, LUA_ICON
    envtool_segment(&TOOL, "blue", &color1(), "LUA_ICON")
}

/// p10k:3003-3057 — prompt_jenv.
fn segment_jenv() -> Option<Vec<Segment>> {
    const TOOL: EnvTool = EnvTool {
        seg: "jenv",
        prefix: "JENV",
        version_file: ".java-version",
        root_default: ".jenv",
        file_prefix: "",
    };
    // p10k:3057 — white bg, red fg, JAVA_ICON
    envtool_segment(&TOOL, "white", "red", "JAVA_ICON")
}

/// p10k:3064-3114 — prompt_plenv.
fn segment_plenv() -> Option<Vec<Segment>> {
    const TOOL: EnvTool = EnvTool {
        seg: "plenv",
        prefix: "PLENV",
        version_file: ".perl-version",
        root_default: ".plenv",
        file_prefix: "",
    };
    // p10k:3114 — blue bg, color1 fg, PERL_ICON
    envtool_segment(&TOOL, "blue", &color1(), "PERL_ICON")
}

/// p10k:2881-2935 — prompt_phpenv.
fn segment_phpenv() -> Option<Vec<Segment>> {
    const TOOL: EnvTool = EnvTool {
        seg: "phpenv",
        prefix: "PHPENV",
        version_file: ".php-version",
        root_default: ".phpenv",
        file_prefix: "",
    };
    // p10k:2935 — magenta bg, color1 fg, PHP_ICON
    envtool_segment(&TOOL, "magenta", &color1(), "PHP_ICON")
}

/// p10k:2823-2876 — prompt_scalaenv.
fn segment_scalaenv() -> Option<Vec<Segment>> {
    const TOOL: EnvTool = EnvTool {
        seg: "scalaenv",
        prefix: "SCALAENV",
        version_file: ".scala-version",
        root_default: ".scalaenv",
        file_prefix: "",
    };
    // p10k:2876 — red bg, color1 fg, SCALA_ICON
    envtool_segment(&TOOL, "red", &color1(), "SCALA_ICON")
}

/// p10k:5575-5588 — `_p9k_haskell_stack_version`: `stack query
/// compiler actual`, stat-cached on the yaml + stack's sqlite dbs.
fn haskell_stack_version(yaml: &Path) -> Option<String> {
    let stack = have_cmd("stack")?;
    let stack_root = envv("STACK_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".stack"));
    let key = format!("haskell_stack {}", yaml.display());
    let files = vec![
        yaml.to_path_buf(),
        stack_root.join("pantry/pantry.sqlite3"),
        stack_root.join("stack.sqlite3"),
    ];
    let vals = match cache_get(&key, &files) {
        Some(v) => v,
        None => {
            // p10k:5578-5586 — STACK_YAML=$1 stack --silent ... query compiler actual
            let out = run_cmd(
                &stack,
                &[
                    "--silent",
                    "--no-install-ghc",
                    "--skip-ghc-check",
                    "--no-terminal",
                    "--color=never",
                    "--lock-file=read-only",
                    "query",
                    "compiler",
                    "actual",
                ],
                false,
                &[("STACK_YAML", &yaml.to_string_lossy())],
            )
            .filter(|(ok, _)| *ok)
            .map(|(_, text)| text)
            .unwrap_or_default();
            cache_set(&key, &files, vec![out])
        }
    };
    vals.into_iter().next().filter(|v| !v.is_empty())
}

/// p10k:5589-5617 — prompt_haskell_stack.
fn segment_haskell_stack() -> Option<Vec<Segment>> {
    if have_cmd("stack").is_none() {
        return hidden(); // init cond p10k:5620 — `'$commands[stack]'`
    }
    let sources = {
        let s = p9k_param_arr("haskell_stack", None, "SOURCES");
        if s.is_empty() {
            vec!["shell".into(), "local".into()] // p10k:7460 — default (shell local)
        } else {
            s
        }
    };
    let has = |w: &str| sources.iter().any(|s| s == w);
    // p10k:7459 — PROMPT_ALWAYS_SHOW default 1
    let always_show = p9k_param_bool("haskell_stack", None, "PROMPT_ALWAYS_SHOW", true);
    let stack_root = envv("STACK_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".stack"));
    let global_yaml = stack_root.join("global-project/stack.yaml");

    let v = if let Some(sy) = envv("STACK_YAML") {
        // p10k:5590-5592 — shell source
        if !has("shell") {
            return hidden();
        }
        haskell_stack_version(Path::new(&sy))
    } else {
        if !has("local") && !has("global") {
            return hidden(); // p10k:5594
        }
        match upfind("stack.yaml") {
            // p10k:5600-5603 — local stack.yaml
            Some(dir) => {
                if !has("local") {
                    return hidden();
                }
                haskell_stack_version(&dir.join("stack.yaml"))
            }
            // p10k:5595-5598 — no local: global, gated on ALWAYS_SHOW
            None => {
                if !always_show || !has("global") {
                    return hidden();
                }
                haskell_stack_version(&global_yaml)
            }
        }
    };
    let Some(v) = v else { return hidden() }; // p10k:5606 — `[[ -n $_p9k__ret ]] || return`
    // p10k:5610-5613 — hide when equal to the global version
    if !always_show {
        if let Some(g) = haskell_stack_version(&global_yaml) {
            if g == v {
                return hidden();
            }
        }
    }
    // p10k:5615 — yellow bg, color1 fg, HASKELL_ICON
    one(seg("haskell_stack", None, "yellow", &color1(), "HASKELL_ICON", esc(&v)))
}

/// p10k:4447-4533 — prompt_kubecontext.
fn segment_kubecontext() -> Option<Vec<Segment>> {
    if have_cmd("kubectl").is_none() {
        return hidden(); // init cond p10k:4536 — `'$commands[kubectl]'`
    }
    // p10k:4448 — cache keyed on the kubeconfig file set:
    // `${(s.:.)${KUBECONFIG:-$HOME/.kube/config}}`
    let cfg_files: Vec<PathBuf> = envv("KUBECONFIG")
        .map(|k| k.split(':').filter(|p| !p.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_else(|| vec![home().join(".kube/config")]);
    let vals = match cache_get("kubecontext", &cfg_files) {
        Some(v) => v,
        None => {
            let parsed = kubectl_current_context().unwrap_or_else(|| vec![String::new(); 10]);
            cache_set("kubecontext", &cfg_files, parsed)
        }
    };
    // vals layout mirrors _p9k_cache_stat_set at p10k:4520:
    // [name, namespace, cluster, user, cloud_name, cloud_account,
    //  cloud_zone, cloud_cluster, text, state]
    let get = |i: usize| vals.get(i).cloned().unwrap_or_default();
    // p10k:4522-4529 — export the P9K_KUBECONTEXT_* params (the user's
    // KUBECONTEXT_DEFAULT_CONTENT_EXPANSION reads them)
    for (i, p) in [
        "P9K_KUBECONTEXT_NAME",
        "P9K_KUBECONTEXT_NAMESPACE",
        "P9K_KUBECONTEXT_CLUSTER",
        "P9K_KUBECONTEXT_USER",
        "P9K_KUBECONTEXT_CLOUD_NAME",
        "P9K_KUBECONTEXT_CLOUD_ACCOUNT",
        "P9K_KUBECONTEXT_CLOUD_ZONE",
        "P9K_KUBECONTEXT_CLOUD_CLUSTER",
    ]
    .iter()
    .enumerate()
    {
        let _ = setsparam(p, &get(i));
    }
    let text = get(8);
    if text.is_empty() {
        return hidden(); // p10k:4530 — `[[ -n $_p9k__cache_val[9] ]] || return`
    }
    let state = get(9);
    // p10k:4531 — magenta bg, white fg, KUBERNETES_ICON
    one(seg(
        "kubecontext",
        if state.is_empty() { None } else { Some(state.as_str()) },
        "magenta",
        "white",
        "KUBERNETES_ICON",
        esc(&text),
    ))
}

/// The anonymous parser inside prompt_kubecontext (p10k:4450-4519):
/// `kubectl config view -o=yaml`, current-context, then the matching
/// contexts[] entry scanned backward for cluster/namespace/user.
fn kubectl_current_context() -> Option<Vec<String>> {
    let kubectl = have_cmd("kubectl")?;
    let (ok, out) = run_cmd(&kubectl, &["config", "view", "-o=yaml"], false, &[])?;
    if !ok {
        return None;
    }
    let unquote = |s: &str| -> String {
        // p10k:4479 — `[[ $name == $~qstr ]] && name=$name[2,-2]`
        if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
            s[1..s.len() - 1].to_string()
        } else {
            s.to_string()
        }
    };
    let lines: Vec<&str> = out.lines().collect();
    // p10k:4455-4457 — exactly one `current-context: ...` line
    let ctx_lines: Vec<&str> = lines
        .iter()
        .filter(|l| l.starts_with("current-context: "))
        .copied()
        .collect();
    if ctx_lines.len() != 1 {
        return None;
    }
    let name = unquote(ctx_lines[0].trim_start_matches("current-context: "));
    let mut namespace = String::new();
    let mut cluster = String::new();
    let mut user = String::new();
    // p10k:4458-4477 — locate `contexts:`, then `  name: $name`, then
    // walk backward to `- context:` collecting the entry's fields
    if let Some(cpos) = lines.iter().position(|l| *l == "contexts:") {
        let tail = &lines[cpos + 1..];
        let name_line_a = format!("  name: {name}");
        let name_line_b = format!("  name: \"{name}\"");
        if let Some(npos) = tail.iter().position(|l| **l == name_line_a || **l == name_line_b) {
            for line in tail[..npos].iter().rev() {
                if *line == "- context:" {
                    break; // p10k:4466-4467
                } else if let Some(v) = line.strip_prefix("    cluster: ") {
                    cluster = unquote(v); // p10k:4468-4470
                } else if let Some(v) = line.strip_prefix("    namespace: ") {
                    namespace = unquote(v); // p10k:4471-4473
                } else if let Some(v) = line.strip_prefix("    user: ") {
                    user = unquote(v); // p10k:4474-4476
                }
            }
        }
    }
    if name.is_empty() {
        return Some(vec![String::new(); 10]);
    }
    if namespace.is_empty() {
        namespace = "default".to_string(); // p10k:4485 — `: ${namespace:=default}`
    }
    // p10k:4486-4507 — recognize GKE / EKS cluster names
    let mut cloud_name = String::new();
    let mut cloud_account = String::new();
    let mut cloud_zone = String::new();
    let mut cloud_cluster = String::new();
    let mut text = String::new();
    let shorten = p9k_param_arr("kubecontext", None, "SHORTEN");
    if let Some(rest) = cluster.strip_prefix("gke_") {
        // p10k:4488-4497 — gke_my-account_us-east1-a_cluster-01
        let parts: Vec<&str> = rest.splitn(3, '_').collect();
        if parts.len() == 3 && parts.iter().all(|p| !p.is_empty()) {
            cloud_name = "gke".into();
            cloud_account = parts[0].into();
            cloud_zone = parts[1].into();
            cloud_cluster = parts[2].into();
            if shorten.iter().any(|s| s == "gke") {
                text = cloud_cluster.clone();
            }
        }
    } else if let Some(rest) = cluster.strip_prefix("arn:aws:eks:") {
        // p10k:4498-4507 — arn:aws:eks:us-east-1:123456789012:cluster/cluster-01
        let parts: Vec<&str> = rest.splitn(3, ':').collect();
        if parts.len() == 3 {
            if let Some(c) = parts[2].strip_prefix("cluster/") {
                cloud_name = "eks".into();
                cloud_zone = parts[0].into();
                cloud_account = parts[1].into();
                cloud_cluster = c.into();
                if shorten.iter().any(|s| s == "eks") {
                    text = cloud_cluster.clone();
                }
            }
        }
    }
    if text.is_empty() {
        // p10k:4508-4513 — context name, plus /namespace unless hidden
        text = name.clone();
        // p10k:7515 — SHOW_DEFAULT_NAMESPACE default 1
        let show_default_ns = p9k_param_bool("kubecontext", None, "SHOW_DEFAULT_NAMESPACE", true);
        if show_default_ns || (namespace != "default" && namespace != name) {
            text.push('/');
            text.push_str(&namespace);
        }
    }
    // p10k:4514-4519 — classes → state
    let state = classes_state("kubecontext", &text).unwrap_or_default();
    Some(vec![
        name, namespace, cluster, user, cloud_name, cloud_account, cloud_zone, cloud_cluster,
        text, state,
    ])
}

/// p10k:4937-4951 — prompt_terraform.
fn segment_terraform() -> Option<Vec<Segment>> {
    if have_cmd("terraform").is_none() {
        return hidden(); // init cond p10k:4954 — `'$commands[terraform]'`
    }
    // p10k:4938-4941 — $TF_WORKSPACE, else ${TF_DATA_DIR:-.terraform}/environment
    let ws = match envv("TF_WORKSPACE") {
        Some(w) => w,
        None => {
            let data_dir = envv("TF_DATA_DIR").unwrap_or_else(|| ".terraform".to_string());
            let dd = Path::new(&data_dir);
            let env_file = if dd.is_absolute() {
                dd.join("environment")
            } else {
                cwd().join(dd).join("environment") // p10k:4940 — `:A` anchors at cwd
            };
            match read_word(&env_file) {
                Some(w) => w,
                None => return hidden(),
            }
        }
    };
    // p10k:4942 — hide empty, and "default" unless SHOW_DEFAULT
    // (default 0, p10k:7544)
    if ws.is_empty()
        || (ws == "default" && !p9k_param_bool("terraform", None, "SHOW_DEFAULT", false))
    {
        return hidden();
    }
    // p10k:4943-4949 — classes → state
    let state = classes_state("terraform", &ws);
    // p10k:4950 — color1 bg, blue fg, TERRAFORM_ICON
    one(seg(
        "terraform",
        state.as_deref(),
        &color1(),
        "blue",
        "TERRAFORM_ICON",
        esc(&ws),
    ))
}

/// p10k:1146-1176 — `_p9k_parse_aws_config`: `[profile X]` sections
/// and their `region = ...` keys.
fn parse_aws_config(cfg: &Path) -> Vec<(String, String)> {
    let Ok(content) = fs::read_to_string(cfg) else { return Vec::new() };
    let mut out = Vec::new();
    let mut profile: Option<String> = None;
    for line in content.lines() {
        let t = line.trim();
        if t == "[default]" || t.starts_with("[default]") && t[9..].trim_start().starts_with('#') {
            // p10k:1159-1161 — example: [default]
            profile = Some("default".to_string());
        } else if let Some(rest) = t.strip_prefix("[profile") {
            // p10k:1162-1164 — example: [profile prod]
            if let Some(name) = rest.strip_suffix(']') {
                let name = name.trim();
                if !name.is_empty() {
                    // `${(Q)match[1]}` — strip one quoting layer
                    profile = Some(name.trim_matches('"').trim_matches('\'').to_string());
                }
            }
        } else if let Some(rest) = t.strip_prefix("region") {
            // p10k:1165-1171 — example: region = eu-west-1 (only kept
            // when a profile header preceded it)
            let rest = rest.trim_start();
            if let Some(val) = rest.strip_prefix('=') {
                if let Some(p) = profile.take() {
                    out.push((p, val.trim().to_string()));
                }
            }
        }
    }
    out
}

/// p10k:1181-1207 — prompt_aws.
fn segment_aws() -> Option<Vec<Segment>> {
    // p10k:1182 — profile precedence: AWS_VAULT, AWSUME_PROFILE,
    // AWS_PROFILE, AWS_DEFAULT_PROFILE (also the init cond, p10k:1210)
    let Some(profile) = envv("AWS_VAULT")
        .or_else(|| envv("AWSUME_PROFILE"))
        .or_else(|| envv("AWS_PROFILE"))
        .or_else(|| envv("AWS_DEFAULT_PROFILE"))
    else {
        return hidden();
    };
    let _ = setsparam("P9K_AWS_PROFILE", &profile); // p10k:1182
    // p10k:1183-1189 — classes → state
    let state = classes_state("aws", &profile);
    // p10k:1191-1203 — region: env, else the profile's region from
    // ${AWS_CONFIG_FILE:-~/.aws/config} (stat-cached); exported for
    // content expansions
    let region = envv("AWS_REGION")
        .or_else(|| envv("AWS_DEFAULT_REGION"))
        .or_else(|| {
            let cfg = envv("AWS_CONFIG_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| home().join(".aws/config"));
            let key = format!("aws_config {}", cfg.display());
            let files = vec![cfg.clone()];
            let vals = match cache_get(&key, &files) {
                Some(v) => v,
                None => {
                    let flat: Vec<String> = parse_aws_config(&cfg)
                        .into_iter()
                        .flat_map(|(p, r)| [p, r])
                        .collect();
                    cache_set(&key, &files, flat)
                }
            };
            let mut it = vals.iter();
            while let (Some(p), Some(r)) = (it.next(), it.next()) {
                if *p == profile {
                    return Some(r.clone());
                }
            }
            None
        });
    match region {
        Some(r) => {
            let _ = setsparam("P9K_AWS_REGION", &r); // p10k:1192/1201
        }
        None => {
            let _ = unsetparam("P9K_AWS_REGION");
        }
    }
    // p10k:1206 — red bg, white fg, AWS_ICON
    one(seg("aws", state.as_deref(), "red", "white", "AWS_ICON", esc(&profile)))
}

/// p10k:1214-1226 — prompt_aws_eb_env.
fn segment_aws_eb_env() -> Option<Vec<Segment>> {
    if have_cmd("eb").is_none() {
        return hidden(); // init cond p10k:1229 — `'$commands[eb]'`
    }
    // p10k:1215-1216 — nearest .elasticbeanstalk dir
    let Some(dir) = upfind(".elasticbeanstalk") else { return hidden() };
    let cfg = dir.join(".elasticbeanstalk/config.yml");
    let key = format!("aws_eb_env {}", dir.display());
    let files = vec![cfg];
    let vals = match cache_get(&key, &files) {
        Some(v) => v,
        None => {
            // p10k:1218-1221 — `eb list`, keep the `* ` (current) line
            let env = have_cmd("eb")
                .and_then(|eb| run_cmd(&eb, &["list"], false, &[]))
                .filter(|(ok, _)| *ok)
                .map(|(_, out)| {
                    out.lines()
                        .find(|l| l.starts_with("* "))
                        .map(|l| l[2..].to_string())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            cache_set(&key, &files, vec![env])
        }
    };
    let env = vals.into_iter().next().unwrap_or_default();
    if env.is_empty() {
        return hidden(); // p10k:1224 — `[[ -n $_p9k__cache_val[1] ]] || return`
    }
    // p10k:1225 — black bg, green fg, AWS_EB_ICON
    one(seg("aws_eb_env", None, "black", "green", "AWS_EB_ICON", esc(&env)))
}

/// p10k:4582-4602 — prompt_azure. p10k shells out to jq (or `az
/// account show`); the same selection — the default subscription's
/// name from azureProfile.json — is done natively.
fn segment_azure() -> Option<Vec<Segment>> {
    if have_cmd("az").is_none() {
        return hidden(); // init cond p10k:4605 — `'$commands[az]'`
    }
    // p10k:4583 — ${AZURE_CONFIG_DIR:-~/.azure}/azureProfile.json
    let cfg = envv("AZURE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".azure"))
        .join("azureProfile.json");
    let key = format!("azure {}", cfg.display());
    let files = vec![cfg.clone()];
    let vals = match cache_get(&key, &files) {
        Some(v) => v,
        None => {
            // p10k:4586 — jq '[.subscriptions[]|select(.isDefault==true)|.name][]|strings'
            let name = fs::read_to_string(&cfg)
                .ok()
                .and_then(|data| {
                    serde_json::from_str::<serde_json::Value>(data.trim_start_matches('\u{feff}'))
                        .ok()
                })
                .and_then(|json| {
                    json.get("subscriptions")?.as_array()?.iter().find_map(|s| {
                        if s.get("isDefault").and_then(|d| d.as_bool()) == Some(true) {
                            s.get("name").and_then(|n| n.as_str()).map(str::to_string)
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_default();
            // p10k:4587 — `name=${name%%$'\n'*}` (first line only)
            let name = name.lines().next().unwrap_or("").to_string();
            cache_set(&key, &files, vec![name])
        }
    };
    let name = vals.into_iter().next().unwrap_or_default();
    if name.is_empty() {
        return hidden(); // p10k:4600 — `[[ -n $_p9k__cache_val[1] ]] || return`
    }
    // p10k:4593-4598 — classes → state
    let state = classes_state("azure", &name);
    // p10k:4601 — blue bg, white fg, AZURE_ICON
    one(seg("azure", state.as_deref(), "blue", "white", "AZURE_ICON", esc(&name)))
}

/// p10k:4608-4661 — prompt_gcloud + `_p9k_gcloud_prefetch`. The async
/// project-NAME fetch (`gcloud projects describe`, worker-based,
/// p10k:4663-4692) is not ported: in zsh the PARTIAL form
/// (account:project_id) is what renders until the worker answers; it
/// renders here always.
fn segment_gcloud() -> Option<Vec<Segment>> {
    if have_cmd("gcloud").is_none() {
        return hidden(); // p10k:4623 / init cond p10k:4658
    }
    // p10k:4624 — `_p9k_read_word ~/.config/gcloud/active_config || return`
    let active_cfg_file = home().join(".config/gcloud/active_config");
    let Some(configuration) = read_word(&active_cfg_file) else {
        return hidden();
    };
    let _ = setsparam("P9K_GCLOUD_CONFIGURATION", &configuration); // p10k:4625
    // p10k:4626-4632 — describe the active configuration, stat-cached
    // on its config file
    let cfg_file = home().join(format!(".config/gcloud/configurations/config_{configuration}"));
    let key = format!("gcloud {configuration}");
    let files = vec![cfg_file];
    let vals = match cache_get(&key, &files) {
        Some(v) => v,
        None => {
            // p10k:4628-4630 — `gcloud config configurations describe
            // $cfg --format=value[separator="\1"](...)`
            let pair = have_cmd("gcloud")
                .and_then(|g| {
                    run_cmd(
                        &g,
                        &[
                            "config",
                            "configurations",
                            "describe",
                            &configuration,
                            "--format=value[separator=\"\u{1}\"](properties.core.account,properties.core.project)",
                        ],
                        false,
                        &[],
                    )
                })
                .filter(|(ok, _)| *ok)
                .map(|(_, out)| out)
                .unwrap_or_default();
            // p10k:4631 — `IFS=$'\1' read account project_id`
            let first_line = pair.lines().next().unwrap_or("");
            let mut it = first_line.splitn(2, '\u{1}');
            let account = it.next().unwrap_or("").to_string();
            let project_id = it.next().unwrap_or("").to_string();
            cache_set(&key, &files, vec![account, project_id])
        }
    };
    let account = vals.first().cloned().unwrap_or_default();
    let project_id = vals.get(1).cloned().unwrap_or_default();
    if !account.is_empty() {
        let _ = setsparam("P9K_GCLOUD_ACCOUNT", &account); // p10k:4634-4636
    }
    if !project_id.is_empty() {
        let _ = setsparam("P9K_GCLOUD_PROJECT_ID", &project_id); // p10k:4637-4639
        // p10k:4639 — deprecated twin, kept for backward compatibility
        let _ = setsparam("P9K_GCLOUD_PROJECT", &project_id);
    }
    // p10k:4610-4613 — GCLOUD_PARTIAL cond: shown when the project
    // name is unknown AND $P9K_GCLOUD_ACCOUNT$P9K_GCLOUD_PROJECT_ID
    // is non-empty
    if account.is_empty() && project_id.is_empty() {
        return hidden();
    }
    // p10k:4610-4614 — blue bg, white fg, GCLOUD_ICON, content
    // '${P9K_GCLOUD_ACCOUNT//\%/%%}:${P9K_GCLOUD_PROJECT_ID//\%/%%}'
    let content = format!("{}:{}", esc(&account), esc(&project_id));
    one(seg("gcloud", Some("PARTIAL"), "blue", "white", "GCLOUD_ICON", content))
}

/// p10k:4695-4721 — prompt_google_app_cred. p10k requires jq (init
/// cond p10k:4724 — `'${GOOGLE_APPLICATION_CREDENTIALS:+$commands[jq]}'`);
/// the JSON is parsed natively instead, so jq presence is not gated on.
fn segment_google_app_cred() -> Option<Vec<Segment>> {
    for p in [
        "P9K_GOOGLE_APP_CRED_TYPE",
        "P9K_GOOGLE_APP_CRED_PROJECT_ID",
        "P9K_GOOGLE_APP_CRED_CLIENT_EMAIL",
    ] {
        let _ = unsetparam(p); // p10k:4696
    }
    let Some(cred_path) = envv("GOOGLE_APPLICATION_CREDENTIALS") else {
        return hidden(); // init cond p10k:4724
    };
    let cred = PathBuf::from(&cred_path);
    let key = format!("google_app_cred {}", cred.display());
    let files = vec![cred.clone()];
    let vals = match cache_get(&key, &files) {
        Some(v) => v,
        None => {
            // p10k:4699-4701 — jq '[.type//"", .project_id//"", .client_email//"", 0][]'
            let fields = fs::read_to_string(&cred)
                .ok()
                .and_then(|data| {
                    serde_json::from_str::<serde_json::Value>(data.trim_start_matches('\u{feff}'))
                        .ok()
                })
                .map(|json| {
                    let f = |k: &str| {
                        json.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
                    };
                    (f("type"), f("project_id"), f("client_email"))
                });
            match fields {
                Some((t, p, e)) => {
                    let text = format!("{t}:{p}:{e}"); // p10k:4702 — `${(j.:.)lines[1,-2]}`
                    // p10k:4703-4709 — classes → state
                    let state = classes_state("google_app_cred", &text).unwrap_or_default();
                    cache_set(&key, &files, vec!["1".into(), t, p, e, text, state])
                }
                None => cache_set(&key, &files, vec!["0".into()]), // p10k:4712
            }
        }
    };
    if vals.first().map(String::as_str) != Some("1") {
        return hidden(); // p10k:4716 — `(( _p9k__cache_val[1] )) || return`
    }
    let get = |i: usize| vals.get(i).cloned().unwrap_or_default();
    let _ = setsparam("P9K_GOOGLE_APP_CRED_TYPE", &get(1)); // p10k:4717-4719
    let _ = setsparam("P9K_GOOGLE_APP_CRED_PROJECT_ID", &get(2));
    let _ = setsparam("P9K_GOOGLE_APP_CRED_CLIENT_EMAIL", &get(3));
    let state = get(5);
    // p10k:4720 — blue bg, white fg, GCLOUD_ICON
    one(seg(
        "google_app_cred",
        if state.is_empty() { None } else { Some(state.as_str()) },
        "blue",
        "white",
        "GCLOUD_ICON",
        esc(&get(4)),
    ))
}

/// p10k:4833-4855 — prompt_nordvpn (Linux only). The status probe
/// speaks raw gRPC over /run/nordvpn/nordvpnd.sock
/// (`_p9k_fetch_nordvpn_status`, p10k:4744-4831 — byte-exact HTTP/2
/// frames + protobuf varint decoding); not ported. The zsh gate is:
/// no socket, no segment — the permanent state on macOS. On a Linux
/// box with the daemon running this hides instead of showing status;
/// logged so the gap is visible.
fn segment_nordvpn() -> Option<Vec<Segment>> {
    if have_cmd("nordvpn").is_none() {
        return hidden(); // init cond p10k:4858 — `'$commands[nordvpn]'`
    }
    // p10k:4835 — `[[ -e /run/nordvpn/nordvpnd.sock ]] || return`
    if !Path::new("/run/nordvpn/nordvpnd.sock").exists() {
        return hidden();
    }
    tracing::debug!(
        target: "p10k",
        "nordvpn daemon socket present but the raw-gRPC status probe is not ported; segment hidden"
    );
    hidden()
}

/// p10k:4859-4861 — prompt_ranger: $RANGER_LEVEL (also the init cond,
/// p10k:4864).
fn segment_ranger() -> Option<Vec<Segment>> {
    let Some(level) = envv("RANGER_LEVEL") else { return hidden() };
    // p10k:4860 — color1 bg, yellow fg, RANGER_ICON, level content
    one(seg("ranger", None, &color1(), "yellow", "RANGER_ICON", esc(&level)))
}

/// p10k:4885-4887 — prompt_nnn: $NNNLVL, hidden when 0 (init cond
/// p10k:4890 — `'${NNNLVL:#0}'`).
fn segment_nnn() -> Option<Vec<Segment>> {
    let Some(level) = envv("NNNLVL").filter(|l| l != "0") else {
        return hidden();
    };
    // p10k:4886 — bg 6, color1 fg, NNN_ICON, level content
    one(seg("nnn", None, "6", &color1(), "NNN_ICON", esc(&level)))
}

// ---------------------------------------------------------------------------
// asdf
// ---------------------------------------------------------------------------

/// p10k:5479-5570 — prompt_asdf. Emits one segment per plugin with a
/// resolved version. Legacy version files (.asdfrc
/// `legacy_version_file = yes` + per-plugin list-legacy-filenames
/// scripts, p10k:5300-5378 / 5385-5409) are not ported — they require
/// executing plugin scripts per prompt-context; .tool-versions (the
/// asdf-native path) is complete.
fn segment_asdf() -> Option<Vec<Segment>> {
    // p10k:5330-5335 — plugins under ${ASDF_DATA_DIR:-~/.asdf}/plugins,
    // installed versions under installs/<plugin>/ (+ implicit "system")
    let root = envv("ASDF_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".asdf"));
    // init cond p10k:5572 — command or function `asdf` (root presence
    // accepted too: shims-only setups expose neither in zshrs's tables)
    if !cmd_or_func("asdf") && !root.is_dir() {
        return hidden();
    }
    let plugins_dir = root.join("plugins");
    let mut plugins: HashMap<String, Vec<String>> = HashMap::new();
    if let Ok(rd) = fs::read_dir(&plugins_dir) {
        for entry in rd.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let plugin = entry.file_name().to_string_lossy().into_owned();
            let mut installed: Vec<String> = Vec::new();
            if let Ok(ird) = fs::read_dir(root.join("installs").join(&plugin)) {
                for ie in ird.flatten() {
                    if ie.path().is_dir() {
                        installed.push(ie.file_name().to_string_lossy().into_owned());
                    }
                }
            }
            installed.push("system".to_string()); // p10k:5335 — `... system)`
            plugins.insert(plugin, installed);
        }
    }
    if plugins.is_empty() {
        return hidden();
    }

    // p10k:5410-5434 — parse one .tool-versions file into a
    // set-if-unset version map: `plugin version...`, comments
    // stripped, the first listed version that is actually installed
    // wins (else the first listed).
    let parse_tool_versions = |file: &Path, versions: &mut HashMap<String, String>| {
        let Ok(content) = fs::read_to_string(file) else { return };
        for line in content.lines() {
            let line = line.trim_end_matches('\r'); // p10k:5417 — `%$'\r'`
            let line = line.split('#').next().unwrap_or(""); // p10k:5417 — `/\#*`
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.len() < 2 {
                continue; // p10k:5421 — `(( $#words > 1 )) || continue`
            }
            let Some(installed) = plugins.get(words[0]) else { continue }; // p10k:5422-5423
            // p10k:5424 — `${${words:1}[(r)$installed]:-$words[2]}`
            let version = words[1..]
                .iter()
                .find(|w| installed.iter().any(|i| i == **w))
                .copied()
                .unwrap_or(words[1]);
            // p10k:5430-5432 — `: ${versions[$plugin]=$version}`
            versions.entry(words[0].to_string()).or_insert_with(|| version.to_string());
        }
    };

    // p10k:5484-5493 — walk cwd→root; the home dir is the local/global
    // boundary (files at/above ~ feed the global map, p10k:5502-5507)
    let mut dirs: Vec<PathBuf> = Vec::new();
    let start = cwd();
    let mut cur: &Path = &start;
    loop {
        dirs.push(cur.to_path_buf());
        match cur.parent() {
            Some(p) => cur = p,
            None => break,
        }
    }
    let h = home();
    if dirs.last() != Some(&h) {
        dirs.push(h.clone()); // p10k:5488-5492 — `dirs+=(~)`
    }
    let mut versions: HashMap<String, String> = HashMap::new();
    let mut local_versions: HashMap<String, String> = HashMap::new();
    let mut has_global = false;
    for dir in &dirs {
        if *dir == h && !has_global {
            // p10k:5502-5507 — everything gathered so far was local
            has_global = true;
            local_versions = std::mem::take(&mut versions);
        }
        let tv = dir.join(".tool-versions");
        if tv.is_file() {
            parse_tool_versions(&tv, &mut versions);
        }
    }
    if !has_global {
        // p10k:5515-5519
        local_versions = std::mem::take(&mut versions);
    }
    // p10k:5521-5523 — $ASDF_DEFAULT_TOOL_VERSIONS_FILENAME is global
    if let Some(f) = envv("ASDF_DEFAULT_TOOL_VERSIONS_FILENAME") {
        let f = PathBuf::from(f);
        if f.is_file() {
            parse_tool_versions(&f, &mut versions);
        }
    }

    // p10k:5525-5567 — one segment per plugin, deterministic order
    // (zsh iterates hash order; sorted here for stable prompts)
    let mut names: Vec<&String> = plugins.keys().collect();
    names.sort();
    let mut out = Vec::new();
    for plugin in names {
        // p10k:5526 — `local upper=${${(U)plugin//-/_}//İ/I}`
        let upper = plugin.replace('-', "_").to_ascii_uppercase();
        // p10k:5527-5532 — per-plugin ASDF_<TOOL>_SOURCES override,
        // else ASDF_SOURCES (p9k_param_arr's probe chain covers both)
        let mut sources = p9k_param_arr("asdf", Some(&upper), "SOURCES");
        if sources.is_empty() {
            sources = vec!["shell".into(), "local".into(), "global".into()]; // p10k:7448
        }
        let has = |w: &str| sources.iter().any(|s| s == w);
        // p10k:5534-5546 — shell (ASDF_<TOOL>_VERSION) > local > global
        let version = if let Some(v) = envv(&format!("ASDF_{upper}_VERSION")) {
            if !has("shell") {
                continue;
            }
            v
        } else if let Some(v) = local_versions.get(plugin) {
            if !has("local") {
                continue;
            }
            v.clone()
        } else if let Some(v) = versions.get(plugin) {
            if !has("global") {
                continue;
            }
            v.clone()
        } else {
            continue;
        };
        // p10k:5548-5554 — equal to global: PROMPT_ALWAYS_SHOW gate
        // (default 0, p10k:7446)
        if Some(&version) == versions.get(plugin)
            && !p9k_param_bool("asdf", Some(&upper), "PROMPT_ALWAYS_SHOW", false)
        {
            continue;
        }
        // p10k:5556-5562 — "system": SHOW_SYSTEM gate (default 1,
        // p10k:7447)
        if version == "system" && !p9k_param_bool("asdf", Some(&upper), "SHOW_SYSTEM", true) {
            continue;
        }
        // p10k:5564-5565 — `_p9k_get_icon $0_$upper ${upper}_ICON $plugin`;
        // green bg, color1 fg, state = plugin upper (drives the user's
        // per-tool ASDF_<TOOL>_FOREGROUND/BACKGROUND overrides)
        let mut s = seg(
            "asdf",
            Some(&upper),
            "green",
            &color1(),
            &format!("{upper}_ICON"),
            esc(&version),
        );
        if s.icon.is_none() {
            // _p9k_get_icon's `$3` fallback: the plugin name itself
            s.icon = Some(plugin.clone());
        }
        out.push(s);
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Env/version-manager segment dispatch. `None` = not this module's
/// segment; `Some(vec![])` = handled, hidden right now.
pub fn build_segment(name: &str) -> Option<Vec<Segment>> {
    match name {
        "direnv" => segment_direnv(),
        "asdf" => segment_asdf(),
        "virtualenv" => segment_virtualenv(),
        "anaconda" => segment_anaconda(),
        "pyenv" => segment_pyenv(),
        "goenv" => segment_goenv(),
        "nodenv" => segment_nodenv(),
        "nvm" => segment_nvm(),
        "nodeenv" => segment_nodeenv(),
        "node_version" => segment_node_version(),
        "go_version" => segment_go_version(),
        "rust_version" => segment_rust_version(),
        "os_icon" => segment_os_icon(),
        "java_version" => segment_java_version(),
        "package" => segment_package(),
        "rbenv" => segment_rbenv(),
        "rvm" => segment_rvm(),
        "fvm" => segment_fvm(),
        "luaenv" => segment_luaenv(),
        "jenv" => segment_jenv(),
        "plenv" => segment_plenv(),
        "phpenv" => segment_phpenv(),
        "scalaenv" => segment_scalaenv(),
        "haskell_stack" => segment_haskell_stack(),
        "kubecontext" => segment_kubecontext(),
        "terraform" => segment_terraform(),
        "aws" => segment_aws(),
        "aws_eb_env" => segment_aws_eb_env(),
        "azure" => segment_azure(),
        "gcloud" => segment_gcloud(),
        "google_app_cred" => segment_google_app_cred(),
        "nordvpn" => segment_nordvpn(),
        "ranger" => segment_ranger(),
        "nnn" => segment_nnn(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_match_star_patterns() {
        assert!(glob_match("*prod*", "us-prod-1"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
        assert!(!glob_match("*prod*", "staging"));
        assert!(glob_match("gke_*", "gke_acct_zone_cluster"));
    }

    #[test]
    fn esc_doubles_percent() {
        assert_eq!(esc("50%"), "50%%");
        assert_eq!(esc("plain"), "plain");
    }

    #[test]
    fn unknown_segment_is_none() {
        assert!(build_segment("no_such_segment_xyz").is_none());
    }

    #[test]
    fn read_pyenv_like_version_file_joins_and_strips() {
        let dir = std::env::temp_dir().join("p10k_segments_env_test");
        let _ = fs::create_dir_all(&dir);
        let f = dir.join("version");
        fs::write(&f, "python-3.11.4 extra\n# comment\n2.7.18\n").unwrap();
        assert_eq!(
            read_pyenv_like_version_file(&f, "python-").as_deref(),
            Some("3.11.4:2.7.18")
        );
        let _ = fs::remove_file(&f);
    }

    #[test]
    fn parse_aws_config_profiles_and_regions() {
        let dir = std::env::temp_dir().join("p10k_segments_env_test");
        let _ = fs::create_dir_all(&dir);
        let f = dir.join("aws_config");
        fs::write(
            &f,
            "[default]\nregion = us-east-1\n[profile prod]\nregion = eu-west-1\n",
        )
        .unwrap();
        let parsed = parse_aws_config(&f);
        assert_eq!(
            parsed,
            vec![
                ("default".to_string(), "us-east-1".to_string()),
                ("prod".to_string(), "eu-west-1".to_string()),
            ]
        );
        let _ = fs::remove_file(&f);
    }

    #[test]
    fn nodenv_transform_passes_system_and_empty() {
        assert_eq!(nodenv_version_transform("system").as_deref(), Some("system"));
        assert_eq!(nodenv_version_transform("").as_deref(), Some(""));
        // nonexistent version with no versions/<v> dir → hide
        assert_eq!(nodenv_version_transform("v99.99.99-does-not-exist"), None);
    }
}
