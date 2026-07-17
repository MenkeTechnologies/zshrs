//! **git-fuzzy** (`status` command), ported from bigH/git-fuzzy to a native
//! zshrs plugin.
//!
//! git-fuzzy is a *self-reentrant* fzf UI: the interactive command builds an
//! fzf whose `--preview` / `--bind` actions call **back** into the tool
//! (`git fuzzy helper <sub> …`) on every keystroke. In bash that re-execs
//! the `git-fuzzy` script and re-sources its library each time — the
//! dispatcher even has "dispatch-aware sourcing" to keep that cheap. That
//! per-keystroke sourcing is exactly the overhead a native host removes.
//!
//! Here every command AND helper is a builtin in this one plugin. fzf's
//! binds reach the helpers through a small generated shim (see
//! [`helper_shim`]) that runs `zshrs -fc 'zmodload -R <self>; gf --helper
//! <sub> <args>'` — one dlopen of an mmap'd dylib instead of sourcing a
//! library tree.
//!
//! This port covers the `status` command end-to-end: the menu, the diff
//! preview, the full-screen inspect view, stage / unstage / discard / amend
//! / patch / commit / edit key-bindings, and the live-reload watcher driven
//! through fzf's `--listen` port.
//!
//! Requires `git` and `fzf` (>= 0.71) on PATH, like git-fuzzy. `delta` /
//! `diff-so-fancy` are used for diff rendering when present.

use std::io::{Read, Write};
use std::os::raw::{c_char, c_int, c_void};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use znative::{declare_plugin, Args, Host};

// ANSI colors (git-fuzzy's $GREEN/$RED/... from load-configs.sh).
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const GRAY: &str = "\x1b[90m";
const WHITE: &str = "\x1b[37m";
const NORMAL: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";

// ============================================================
// config (env / shell-param overridable, with git-fuzzy's defaults)
// ============================================================

/// fzf bind keys must be lowercase (`alt-m`), the labels shown to the user
/// are the configured case (`Alt-M`).
struct Keys {
    amend: String,
    add: String,
    add_patch: String,
    edit: String,
    commit: String,
    reset: String,
    discard: String,
    inspect: String,
    select_all: String,
    select_none: String,
    wrap: String,
    size_inc: String,
    size_dec: String,
}

fn cfg(host: &Host, key: &str, default: &str) -> String {
    host.getvar(key)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

impl Keys {
    fn new(host: &Host) -> Self {
        Keys {
            amend: cfg(host, "GIT_FUZZY_STATUS_AMEND_KEY", "Alt-M"),
            add: cfg(host, "GIT_FUZZY_STATUS_ADD_KEY", "Alt-S"),
            add_patch: cfg(host, "GIT_FUZZY_STATUS_ADD_PATCH_KEY", "Alt-P"),
            edit: cfg(host, "GIT_FUZZY_STATUS_EDIT_KEY", "Alt-E"),
            commit: cfg(host, "GIT_FUZZY_STATUS_COMMIT_KEY", "Alt-C"),
            reset: cfg(host, "GIT_FUZZY_STATUS_RESET_KEY", "Alt-R"),
            discard: cfg(host, "GIT_FUZZY_STATUS_DISCARD_KEY", "Alt-U"),
            inspect: cfg(host, "GIT_FUZZY_INSPECT_KEY", "Alt-I"),
            select_all: cfg(host, "GIT_FUZZY_SELECT_ALL_KEY", "Alt-A"),
            select_none: cfg(host, "GIT_FUZZY_SELECT_NONE_KEY", "Alt-D"),
            wrap: cfg(host, "GIT_FUZZY_PREVIEW_WRAP_KEY", "Alt-W"),
            size_inc: cfg(host, "GIT_FUZZY_PREVIEW_SIZE_INCREASE_KEY", "Alt-="),
            size_dec: cfg(host, "GIT_FUZZY_PREVIEW_SIZE_DECREASE_KEY", "Alt--"),
        }
    }
}

fn lc(s: &str) -> String {
    s.to_lowercase()
}

// ============================================================
// git / process helpers
// ============================================================

fn in_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Capture stdout of a command (as a String), stderr inherited.
fn capture(prog: &str, args: &[&str]) -> String {
    Command::new(prog)
        .args(args)
        .stderr(Stdio::inherit())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Run a command inheriting all stdio (its output goes straight to the
/// terminal / fzf).
fn run(prog: &str, args: &[&str]) -> bool {
    Command::new(prog)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn tool_exists(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ============================================================
// terminal geometry (core.sh's is_vertical / preview-window math,
// ported from the `bc` formulas to integer arithmetic)
// ============================================================

fn term_size() -> (i64, i64) {
    let cols = capture("tput", &["cols"]).trim().parse().unwrap_or(80);
    let lines = capture("tput", &["lines"]).trim().parse().unwrap_or(24);
    (cols.max(1), lines.max(1))
}

fn is_vertical(w: i64, h: i64) -> bool {
    // GF_VERTICAL_THRESHOLD = 2.0  ->  w/h < 2.0  ->  w*10 < h*20
    w * 10 < h * 20
}

/// git-fuzzy's preview-window `<dir>:<pct>%` for the current terminal.
fn preview_window(w: i64, h: i64, hidden: bool) -> String {
    let clamp = |v: i64| v.clamp(50, 80);
    let (dir, pct) = if is_vertical(w, h) {
        // max(50, min(80, 100 - ((4000 + 5*H) / H)))
        ("bottom", clamp(100 - ((4000 + 5 * h) / h)))
    } else {
        // max(50, min(80, 100 - ((7000 + 11*W) / W)))
        ("right", clamp(100 - ((7000 + 11 * w) / w)))
    };
    let vis = if hidden { "hidden" } else { "nohidden" };
    format!("--preview-window={dir}:{pct}%:{vis}")
}

fn small_screen(w: i64, h: i64) -> bool {
    if is_vertical(w, h) {
        h <= 60
    } else {
        h <= 30
    }
}

// ============================================================
// diff rendering (core.sh gf_diff_renderer): delta -> diff-so-fancy -> cat
// ============================================================

/// Pipe `input` through the best available diff renderer, return the
/// rendered text. Mirrors gf_diff_renderer's tool preference.
fn render_diff(input: &str, preview_cols: Option<i64>) -> String {
    let piped = |prog: &str, args: &[&str]| -> Option<String> {
        let mut child = Command::new(prog)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdin = child.stdin.take()?;
        let owned = input.to_string();
        let w = std::thread::spawn(move || {
            let _ = stdin.write_all(owned.as_bytes());
        });
        let out = child.wait_with_output().ok()?;
        let _ = w.join();
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    if tool_exists("delta") {
        let cols = preview_cols.map(|c| c.to_string());
        let mut args = vec!["--paging=never"];
        if let Some(ref c) = cols {
            args.push("--width");
            args.push(c);
        }
        if let Some(s) = piped("delta", &args) {
            return s;
        }
    }
    if tool_exists("diff-so-fancy") {
        if let Some(s) = piped("diff-so-fancy", &[]) {
            return s;
        }
    }
    input.to_string()
}

// ============================================================
// self-location + the fzf->helper shim
// ============================================================

/// Absolute path of THIS dylib, via `dladdr` on one of our own functions.
fn self_dylib() -> Option<String> {
    #[repr(C)]
    struct DlInfo {
        dli_fname: *const c_char,
        dli_fbase: *mut c_void,
        dli_sname: *const c_char,
        dli_saddr: *mut c_void,
    }
    extern "C" {
        fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
    }
    let marker = self_dylib as *const () as *const c_void;
    let mut info: DlInfo = unsafe { std::mem::zeroed() };
    if unsafe { dladdr(marker, &mut info) } == 0 || info.dli_fname.is_null() {
        return None;
    }
    let raw = unsafe { std::ffi::CStr::from_ptr(info.dli_fname) }
        .to_string_lossy()
        .into_owned();
    Some(
        std::fs::canonicalize(&raw)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(raw),
    )
}

/// The zshrs binary running this plugin.
fn zshrs_bin() -> String {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "zshrs".to_string())
}

/// Write (once) and return the path to the shim fzf binds invoke. The shim
/// forwards its args to `gf --helper …` in a fresh zshrs that has this
/// plugin loaded. Using a shim keeps the fzf bind strings free of the
/// nested quoting a direct `zshrs -fc '…'` would need.
fn helper_shim() -> Option<&'static str> {
    static SHIM: OnceLock<Option<String>> = OnceLock::new();
    SHIM.get_or_init(|| {
        let self_path = self_dylib()?;
        let zshrs = zshrs_bin();
        let home = std::env::var("HOME").ok()?;
        let dir = format!("{home}/.cache/zshrs");
        let _ = std::fs::create_dir_all(&dir);
        let shim = format!("{dir}/gf-helper.sh");
        // Pass the dylib path as `$0` (not embedded in the -c script) so a
        // path with spaces survives — the script stays one clean literal
        // single-quoted string. `"$@"` carries fzf's (already shell-quoted)
        // field substitutions through unchanged.
        let body = format!(
            "#!/bin/sh\nexec {zshrs} -fc 'zmodload -R \"$0\" 2>/dev/null; gf --helper \"$@\"' {self} \"$@\"\n",
            zshrs = shell_quote(&zshrs),
            self = shell_quote(&self_path),
        );
        std::fs::write(&shim, body).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755));
        }
        Some(shim)
    })
    .as_deref()
}

/// Minimal POSIX single-quote for embedding a path in the shim.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `<shim> <sub> <fzf-args…>` — the string an fzf action runs.
fn helper_cmd(sub: &str, fzf_args: &str) -> String {
    let shim = helper_shim().unwrap_or("gf-helper-unavailable");
    if fzf_args.is_empty() {
        format!("{shim} {sub}")
    } else {
        format!("{shim} {sub} {fzf_args}")
    }
}

// ============================================================
// the `gf` builtin: dispatch
// ============================================================

fn gf(host: &Host, args: &Args) -> c_int {
    let rest: Vec<&str> = args.rest().iter().map(String::as_str).collect();
    match rest.as_slice() {
        [] | ["menu"] => gf_status(host), // menu not ported yet -> status
        ["--helper", sub, tail @ ..] => helper_dispatch(host, sub, tail),
        ["status", ..] => gf_status(host),
        [cmd, ..] => {
            host.print(&format!("gf: `{cmd}` not ported (this example covers `status`)\n"));
            1
        }
    }
}

// ============================================================
// interactive `status` (modules/status.sh gf_status + gf_fzf_status
// + gf_status_interpreter)
// ============================================================

fn gf_status(host: &Host) -> c_int {
    if !in_git_repo() {
        host.print("not in git repo\n");
        return 1;
    }
    if capture("git", &["status", "-s"]).trim().is_empty() {
        // nothing to commit, working tree clean
        return 1;
    }

    let keys = Keys::new(host);
    let (w, h) = term_size();
    let menu = status_menu_content();

    // --- build the fzf command (gf_fzf + gf_fzf_status) ---
    let shim = helper_shim().unwrap_or("gf-helper-unavailable");
    let header = if small_screen(w, h) {
        String::new()
    } else {
        status_header(&keys)
    };
    let reload = format!("reload-sync({})", helper_cmd("status_menu_content", ""));
    let action_reload = format!("{reload}+clear-multi");
    let expect = format!("{},{},{}", lc(&keys.edit), lc(&keys.commit), lc(&keys.add_patch));

    let inspect_bind = format!(
        "{}:execute({})",
        lc(&keys.inspect),
        helper_cmd("status_diff", "{1} {2} {4}")
    );

    let mut fzf = Command::new("fzf");
    fzf.env("GIT_OPTIONAL_LOCKS", "0")
        .arg("--ansi")
        .arg("--no-sort")
        .arg("--no-info")
        .arg("--multi")
        .arg("--track")
        .arg("--listen")
        .arg("--header-lines=2")
        .arg("--nth=2")
        .arg("--id-nth=2..")
        .arg(preview_window(w, h, false))
        .arg("--header")
        .arg(&header)
        .arg("--expect")
        .arg(&expect)
        .arg(format!("--preview={}", helper_cmd("status_preview_content", "{1} {2} {4}")))
        .arg(format!("--bind={}:toggle-preview-wrap", lc(&keys.wrap)))
        .arg(format!("--bind={}:select-all", lc(&keys.select_all)))
        .arg(format!("--bind={}:deselect-all", lc(&keys.select_none)))
        .arg(format!(
            "--bind={}:transform({})",
            lc(&keys.size_inc),
            helper_cmd("preview_resize", "increase")
        ))
        .arg(format!(
            "--bind={}:transform({})",
            lc(&keys.size_dec),
            helper_cmd("preview_resize", "decrease")
        ))
        .arg(format!(
            "--bind=start:execute-silent({} $FZF_PORT > /dev/null 2>&1 &)",
            helper_cmd("status_watch", "")
        ))
        .arg(format!("--bind={inspect_bind}"))
        .arg(format!(
            "--bind={}:execute-silent({} {{+2..}})+{action_reload}+down",
            lc(&keys.amend),
            helper_cmd("status_amend", "")
        ))
        .arg(format!(
            "--bind={}:execute-silent({} {{+2..}})+{action_reload}+down",
            lc(&keys.add),
            helper_cmd("status_add", "")
        ))
        .arg(format!(
            "--bind={}:execute-silent({} {{+2..}})+{action_reload}+down",
            lc(&keys.reset),
            helper_cmd("status_reset", "")
        ))
        .arg(format!(
            "--bind={}:execute-silent({} {{+2..}})+{reload}",
            lc(&keys.discard),
            helper_cmd("status_discard", "")
        ));
    let _ = shim;

    fzf.stdin(Stdio::piped()).stdout(Stdio::piped());
    let Ok(mut child) = fzf.spawn() else {
        host.print("gf: failed to launch fzf (is it installed, >= 0.71?)\n");
        return 1;
    };
    // Feed the menu on a thread; capture the selection.
    let mut stdin = child.stdin.take().unwrap();
    let owned = menu;
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(owned.as_bytes());
    });
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return 1,
    };
    let _ = writer.join();
    let selection = String::from_utf8_lossy(&out.stdout).into_owned();

    status_interpret(host, &keys, &selection)
}

/// The 2-line header echoed above the list, plus the shortcut legend.
fn status_header(keys: &Keys) -> String {
    let editor = std::env::var("EDITOR").unwrap_or_default();
    format!(
        "Type to filter. {WHITE}Enter{NORMAL} to {GREEN}ACCEPT{NORMAL}\n\n\
         {GRAY}-- (*) editor: {NORMAL}{editor}\n\
         {GREEN}amend{NORMAL} {WHITE}{amend}{NORMAL}  {GREEN}stage -p{NORMAL} {WHITE}{patch}{NORMAL}  * {GREEN}edit{NORMAL} {WHITE}{edit}{NORMAL}\n\
         {GREEN}all{NORMAL} {WHITE}{all}{NORMAL}  {GREEN}stage{NORMAL} {WHITE}{add}{NORMAL}  {RED}discard{NORMAL} {WHITE}{discard}{NORMAL}\n\
         {GREEN}none{NORMAL} {WHITE}{none}{NORMAL}  {GREEN}reset{NORMAL} {WHITE}{reset}{NORMAL}  * {RED}commit{NORMAL} {WHITE}{commit}{NORMAL}",
        amend = keys.amend, patch = keys.add_patch, edit = keys.edit,
        all = keys.select_all, add = keys.add, discard = keys.discard,
        none = keys.select_none, reset = keys.reset, commit = keys.commit,
    )
}

/// gf_status_interpreter: `--expect` key on line 1 → run edit/commit/patch;
/// otherwise print the selected file paths.
fn status_interpret(host: &Host, keys: &Keys, selection: &str) -> c_int {
    let mut lines = selection.lines();
    let head = lines.next().unwrap_or("").trim();
    let tail: Vec<&str> = lines.collect();
    // strip the "XY " status prefix (cut -c4-) and rename source.
    let files: Vec<String> = tail
        .iter()
        .filter(|l| l.len() > 3)
        .map(|l| strip_rename(&l[3..]))
        .collect();

    let head_lc = head.to_lowercase();
    if head_lc == lc(&keys.edit) {
        if let Some(f) = files.first() {
            return helper_status_edit(host, &[f.as_str()]);
        }
        0
    } else if head_lc == lc(&keys.commit) {
        helper_status_commit(host)
    } else if head_lc == lc(&keys.add_patch) {
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        helper_status_add_patch(host, &refs)
    } else {
        for f in &files {
            println!("{f}");
        }
        0
    }
}

/// Remove a `X -> ` rename source, leaving the destination path.
fn strip_rename(s: &str) -> String {
    match s.rsplit_once(" -> ") {
        Some((_, dst)) => dst.to_string(),
        None => s.to_string(),
    }
}

// ============================================================
// helper dispatch (the `gf --helper <sub>` subcommands git-fuzzy invokes
// from fzf; here reached via the shim in a fresh zshrs)
// ============================================================

fn helper_dispatch(host: &Host, sub: &str, args: &[&str]) -> c_int {
    match sub {
        "status_menu_content" => {
            print!("{}", status_menu_content());
            0
        }
        "status_preview_content" => helper_status_preview(host, args),
        "status_diff" => helper_status_diff(args),
        "status_add" => helper_status_add(args),
        "status_reset" => helper_status_reset(args),
        "status_discard" => helper_status_discard(args),
        "status_amend" => helper_status_amend(),
        "status_add_patch" => helper_status_add_patch(host, args),
        "status_commit" => helper_status_commit(host),
        "status_edit" => helper_status_edit(host, args),
        "status_watch" => helper_status_watch(args),
        "preview_resize" => helper_preview_resize(args),
        other => {
            host.print(&format!("gf helper `{other}` not found\n"));
            1
        }
    }
}

/// gf_helper_status_menu_content: `$ git status --short` header + list.
/// Uncolored list so fzf field placeholders ({1}/{2}/{4}) stay clean —
/// git-fuzzy colors it and documents that whitespace paths aren't
/// supported; we keep the same path limitation and trade the status-column
/// color for robust field extraction.
fn status_menu_content() -> String {
    let list = capture("git", &["status", "--short"]);
    format!("{GRAY}{BOLD}$ {CYAN}{BOLD}git status --short{NORMAL}\n\n{list}")
}

fn preview_cols_env() -> Option<i64> {
    std::env::var("FZF_PREVIEW_COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
}

/// gf_helper_status_preview_content STATUS FILE RENAMED.
fn helper_status_preview(_host: &Host, args: &[&str]) -> c_int {
    let status = args.first().copied().unwrap_or("");
    let file = args.get(1).copied().unwrap_or("");
    let renamed = args.get(2).copied().unwrap_or("");
    if file.is_empty() {
        return 0;
    }
    if status == "??" {
        // untracked: show the file/dir directly (bat/eza when available).
        let meta = std::fs::metadata(file);
        if meta.map(|m| m.is_dir()).unwrap_or(false) {
            let lister = if tool_exists("eza") { "eza" } else { "ls" };
            print!("{GRAY}{BOLD}$ {CYAN}{BOLD}{lister} -l{NORMAL}\n\n");
            let _ = run(lister, &["-l", "--color=always", file]);
        } else {
            let (cat, args2): (&str, Vec<&str>) = if tool_exists("bat") {
                ("bat", vec!["--color=always", file])
            } else {
                ("cat", vec![file])
            };
            print!("{GRAY}{BOLD}$ {CYAN}{BOLD}{cat}{NORMAL}\n\n");
            let _ = run(cat, &args2);
        }
        return 0;
    }
    // tracked: git diff HEAD -M -- <file> [renamed], through the renderer.
    let mut da = vec!["-c", "color.ui=always", "diff", "HEAD", "-M", "--", file];
    if !std::path::Path::new(file).exists() && !renamed.is_empty() {
        da.push(renamed);
    }
    let raw = capture("git", &da);
    print!(
        "{GRAY}{BOLD}$ {CYAN}{BOLD}git diff HEAD -M -- {file}{NORMAL}\n\n{}",
        render_diff(&raw, preview_cols_env())
    );
    0
}

/// gf_helper_status_diff: the Alt-I full-screen inspect view (via less).
fn helper_status_diff(args: &[&str]) -> c_int {
    let status = args.first().copied().unwrap_or("");
    let file = args.get(1).copied().unwrap_or("");
    let renamed = args.get(2).copied().unwrap_or("");
    if file.is_empty() {
        return 0;
    }
    let content = if status == "??" {
        std::fs::read_to_string(file).unwrap_or_default()
    } else {
        let mut da = vec!["--no-pager", "-c", "color.ui=always", "diff", "HEAD", "-M", "--", file];
        if !std::path::Path::new(file).exists() && !renamed.is_empty() {
            da.push(renamed);
        }
        render_diff(&capture("git", &da), None)
    };
    // pipe into `less -R` for a scrollable full-screen view.
    if let Ok(mut child) = Command::new("less")
        .args(["-R", "-K", "-+F"])
        .stdin(Stdio::piped())
        .spawn()
    {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(content.as_bytes());
        }
        let _ = child.wait();
    } else {
        print!("{content}");
    }
    0
}

/// Flatten fzf `{+2..}` field args into a path list, dropping rename
/// sources (git-fuzzy's `sed 's/[^ ]* -> //g'`).
fn paths_from_fields(args: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut it = args.iter().peekable();
    while let Some(&tok) = it.next() {
        if tok == "->" {
            out.pop(); // drop the rename source just pushed
            if let Some(&dst) = it.next() {
                out.push(dst.to_string());
            }
        } else {
            out.push(tok.to_string());
        }
    }
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

fn helper_status_add(args: &[&str]) -> c_int {
    let paths = paths_from_fields(args);
    let mut a = vec!["add", "--"];
    a.extend(paths.iter().map(String::as_str));
    run("git", &a);
    0
}

fn helper_status_reset(args: &[&str]) -> c_int {
    let paths = paths_from_fields(args);
    let mut a = vec!["reset", "--"];
    a.extend(paths.iter().map(String::as_str));
    run("git", &a);
    0
}

fn helper_status_amend() -> c_int {
    run("git", &["commit", "--amend", "--reuse-message=HEAD"]);
    0
}

fn helper_status_discard(args: &[&str]) -> c_int {
    let paths = paths_from_fields(args);
    if paths.is_empty() {
        return 0;
    }
    // tracked -> checkout HEAD; untracked -> rm -rf.
    let first = &paths[0];
    let tracked = Command::new("git")
        .args(["ls-files", "--error-unmatch", first])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if tracked {
        let mut a = vec!["checkout", "HEAD", "--"];
        a.extend(paths.iter().map(String::as_str));
        run("git", &a);
    } else {
        for p in &paths {
            let _ = std::fs::remove_dir_all(p).or_else(|_| std::fs::remove_file(p));
        }
    }
    0
}

fn helper_status_add_patch(host: &Host, args: &[&str]) -> c_int {
    if args.is_empty() {
        host.print("tried to git add --patch with no file(s)\n");
        return 1;
    }
    let mut a = vec!["add", "--patch", "--"];
    a.extend(args.iter().copied());
    run("git", &a); // inherits /dev/tty via inherited stdio
    if !capture("git", &["status", "-s"]).trim().is_empty() {
        return gf_status(host); // loop back into the status view
    }
    0
}

fn helper_status_commit(host: &Host) -> c_int {
    run("git", &["commit"]);
    if !capture("git", &["status", "-s"]).trim().is_empty() {
        return gf_status(host);
    }
    0
}

fn helper_status_edit(host: &Host, args: &[&str]) -> c_int {
    if args.is_empty() {
        host.print("tried to EDIT in status with no file(s)\n");
        return 1;
    }
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    run(&editor, args);
    0
}

/// gf_helper_preview_resize increase|decrease -> a fzf `change-preview-window`
/// action string on stdout (consumed by the `transform(...)` bind).
fn helper_preview_resize(args: &[&str]) -> c_int {
    let action = args.first().copied().unwrap_or("");
    let cur: i64 = std::env::var("FZF_PREVIEW_COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let total: i64 = std::env::var("FZF_COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if cur <= 0 || total <= 0 {
        return 0;
    }
    let step = 5;
    let mut next = match action {
        "increase" => cur + step,
        "decrease" => cur - step,
        _ => return 0,
    };
    next = next.clamp(1, total);
    println!("change-preview-window:{next}");
    0
}

/// gf_helper_status_watch PORT — live-reload watcher. git-fuzzy watches the
/// filesystem (fswatch/inotifywait) and POSTs `reload-sync(...)` to fzf's
/// `--listen` port. This port polls `git status` (+ HEAD) and POSTs the
/// same reload when it changes; it self-terminates when the POST fails
/// (fzf gone).
fn helper_status_watch(args: &[&str]) -> c_int {
    let Some(port) = args.first().and_then(|s| s.parse::<u16>().ok()) else {
        return 0;
    };
    if std::env::var("GF_STATUS_WATCH").as_deref() == Ok("0") {
        return 0;
    }
    let action = format!("reload-sync({})", helper_cmd("status_menu_content", ""));
    let mut last = state_hash();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let now = state_hash();
        if now == last {
            continue;
        }
        last = now;
        if !fzf_post(port, &action) {
            break; // fzf exited
        }
    }
    0
}

/// Cheap change token: `git status --porcelain` + HEAD.
fn state_hash() -> u64 {
    let s = capture("git", &["status", "--porcelain"]) + &capture("git", &["rev-parse", "HEAD"]);
    // FNV-1a
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// POST an action to fzf's `--listen` port (replaces the shell's `curl`).
fn fzf_post(port: u16, action: &str) -> bool {
    use std::net::TcpStream;
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let req = format!(
        "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        action.len(),
        action
    );
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 64];
    let _ = s.read(&mut buf); // best-effort; connection success == fzf alive
    true
}

declare_plugin! {
    name: "git-fuzzy",
    version: "0.1.0",
    builtins: {
        "gf"        => gf,
        "git-fuzzy" => gf,
    },
}
