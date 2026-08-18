//! zshrs - The most powerful shell ever created
//!
//! A drop-in zsh replacement that combines:
//! - Full bash/zsh script compatibility  
//! - Fish-quality completions with SQLite indexing
//! - Fish-style syntax highlighting and autosuggestions
//!
//! Copyright (C) 2026 MenkeTechnologies
//! License: GPL-2.0 (incorporates code from fish-shell)

use std::env;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use zsh::vm_helper::ShellExecutor;
use zsh::zwc;

use zsh::compsys::cache::CompsysCache;
// CompletionGroup / Completion / CompletionState / do_completion /
// MenuState / MenuAction were deleted with completion.rs + compcore.rs
// + menu.rs. Stub locally so this bin still compiles; real menu +
// completion assembly now live in src/ported/zle/{compcore,complist}.rs.
// The stubs aren't reached by the live REPL code path — `dead_code`
// is suppressed wholesale on the vestigial block.

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
struct CompsysCompletion {
    pub disp: Option<String>,
    pub desc: Option<String>,
}
#[allow(dead_code)]
impl CompsysCompletion {
    fn new(_s: impl AsRef<str>) -> Self {
        Self::default()
    }
}
#[allow(unused_imports)]
use CompsysCompletion as Completion;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
enum MenuAction {
    #[default]
    Next,
    Prev,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
}
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
struct MenuState;
#[allow(dead_code)]
impl MenuState {
    fn new() -> Self {
        Self
    }
    fn start(&mut self) {}
    fn stop(&mut self) {}
    fn cols(&self) -> usize {
        0
    }
    fn selected_index(&self) -> Option<usize> {
        None
    }
    fn set_term_size(&mut self, _w: usize, _h: usize) {}
    fn set_available_rows(&mut self, _r: usize) {}
    fn process_action(&mut self, _a: MenuAction) -> MenuResult {
        MenuResult
    }
    fn render(&self) -> MenuRendering {
        MenuRendering::default()
    }
}
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
struct MenuRendering {
    lines: Vec<MenuLine>,
}
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
struct MenuLine {
    content: String,
}
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
struct MenuResult;

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
struct CompletionGroup {
    pub matches: Vec<CompsysCompletion>,
}
#[allow(dead_code)]
impl CompletionGroup {
    fn new(_name: impl AsRef<str>) -> Self {
        Self::default()
    }
    fn add(&mut self, c: CompsysCompletion) {
        self.matches.push(c);
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
struct CompletionState;
#[allow(dead_code)]
impl CompletionState {
    fn new() -> Self {
        Self
    }
    fn from_line(_line: &str, _cursor: usize) -> Self {
        Self
    }
    fn add_match(&mut self, _c: CompsysCompletion, _group: Option<&str>) {}
    fn begin_group(&mut self, _name: &str, _sorted: bool) {}
    fn end_group(&mut self) {}
}

#[allow(dead_code)]
fn do_completion(
    _line: &str,
    _cursor: usize,
    _state: &mut CompletionState,
    _custom: impl FnOnce(&mut CompletionState),
) -> usize {
    0
}

/// Print help message identical to zsh --help
/// Render an LSP hover card (`**name** — _kind_\n\n<body>`) as a
/// colored terminal page. Matches the `stryke docs NAME` shape:
///
/// ```text
///   CYAN  name             NORMAL
///   DIM   ───────────────  NORMAL
///   DIM   zsh keyword      NORMAL    (or whatever kind)
///
///   body paragraph 1, wrapped to ~term width…
///
///       GREEN indented code block GREEN  (code-fence or 4-sp indent)
/// ```
///
/// Inline backticks → cyan; `**bold**` → ANSI bold (kept readable when
/// `color=false`, which strips all escapes and leaves plain markdown).
///
/// Called only from `--docs NAME`. The IntelliJ tool window passes
/// `--color never` (or just consumes the raw `lsp::lookup_doc` output
/// directly via Bash's `output.stdout`), so the popup keeps its native
/// rendering — coloring is exclusively for terminal users.
fn render_doc_card(name: &str, card: &str, color: bool) -> String {
    let (cyan, green, dim, bold, reset) = if color {
        ("\x1b[36m", "\x1b[32m", "\x1b[2m", "\x1b[1m", "\x1b[0m")
    } else {
        ("", "", "", "", "")
    };

    // Split heading from body. `lookup_doc` always emits
    // `**name** — _kind_\n\n<body>` (or just the heading with no body
    // for the bare-stub fallback path). The kind label sits between
    // `_..._` markers on the first line.
    let (heading, body) = card.split_once("\n\n").unwrap_or((card, ""));
    // Heading shape: `**NAME** — _kind_`. Split on the em-dash
    // separator so an underscore inside NAME (e.g. `AUTO_CD`) doesn't
    // get picked up as the kind boundary.
    let kind = heading
        .rsplit_once(" — ")
        .map(|(_, k)| k.trim().trim_matches('_').to_string())
        .unwrap_or_default();

    let width = term_width().saturating_sub(4).max(40);
    let sep_len = name.chars().count().max(20).min(width);

    let mut out = String::with_capacity(card.len() + 256);
    out.push_str(&format!("  {cyan}{name}{reset}\n"));
    out.push_str(&format!("  {dim}{}{reset}\n", "─".repeat(sep_len)));
    if !kind.is_empty() {
        out.push_str(&format!("  {dim}{kind}{reset}\n"));
    }
    out.push('\n');

    let mut in_fence = false;
    for line in body.split('\n') {
        // Fenced code block (``` … ```)
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            out.push_str(&format!("  {green}  {line}{reset}\n"));
            continue;
        }
        // 4+-space indent → markdown code block.
        if line.starts_with("    ") {
            out.push_str(&format!("  {green}{line}{reset}\n"));
            continue;
        }
        if line.trim().is_empty() {
            out.push('\n');
            continue;
        }
        // Prose — colorize inline backtick spans, render `**bold**`,
        // drop `_underscores_` quietly (no good ANSI italic on every
        // terminal). Word-wrap to the visible width.
        let rendered = render_inline_md(line, cyan, bold, reset);
        for wrapped in word_wrap(&rendered, width) {
            out.push_str(&format!("  {wrapped}\n"));
        }
    }
    out
}

/// Replace markdown-ish inline markup with terminal styling.
fn render_inline_md(line: &str, cyan: &str, bold: &str, reset: &str) -> String {
    let mut out = String::with_capacity(line.len() + 32);
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut in_tick = false;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '`' {
            if in_tick {
                out.push_str(reset);
            } else {
                out.push_str(cyan);
            }
            in_tick = !in_tick;
            i += 1;
            continue;
        }
        // **bold** — only when matched closer exists on the same line.
        if c == '*' && i + 1 < bytes.len() && bytes[i + 1] as char == '*' {
            if let Some(end) = find_bytes(&bytes[i + 2..], b"**") {
                let inner = std::str::from_utf8(&bytes[i + 2..i + 2 + end]).unwrap_or("");
                out.push_str(bold);
                out.push_str(inner);
                out.push_str(reset);
                i += 2 + end + 2;
                continue;
            }
        }
        // `_emph_` — strip the underscores in colored mode (no portable
        // italic); leave them in plain mode so the markdown stays valid.
        if c == '_' && !cyan.is_empty() && (i == 0 || !(bytes[i - 1] as char).is_alphanumeric()) {
            // Look ahead for a matching `_` not bounded by alphanum on
            // its right side.
            if let Some(rel) = find_word_close_underscore(&bytes[i + 1..]) {
                let inner = std::str::from_utf8(&bytes[i + 1..i + 1 + rel]).unwrap_or("");
                // Underline if supported; cheap unicode-clean fallback:
                // just emit dim text.
                out.push_str("\x1b[3m");
                out.push_str(inner);
                out.push_str(reset);
                i += 1 + rel + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn find_word_close_underscore(rest: &[u8]) -> Option<usize> {
    // Find the next `_` whose right side is non-alphanumeric (so
    // `name_with_underscores` doesn't get falsely emphasized).
    for (i, b) in rest.iter().enumerate() {
        if *b == b'_' {
            let after = rest.get(i + 1).map(|c| *c as char);
            if after.map(|c| !c.is_alphanumeric()).unwrap_or(true) {
                return Some(i);
            }
        }
    }
    None
}

/// Greedy word-wrap that respects ANSI escapes. Counts visible width
/// (chars outside `\x1b[…m` runs) so colored spans don't blow up the
/// budget.
fn word_wrap(line: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_vis = 0usize;
    for word in line.split_inclusive(char::is_whitespace) {
        let vis = visible_width(word);
        if cur_vis + vis > max && !cur.trim().is_empty() {
            out.push(std::mem::take(&mut cur).trim_end().to_string());
            cur_vis = 0;
        }
        cur.push_str(word);
        cur_vis += vis;
    }
    if !cur.is_empty() {
        out.push(cur.trim_end().to_string());
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Char count, ignoring ANSI `\x1b[…m` escape sequences.
fn visible_width(s: &str) -> usize {
    let mut n = 0usize;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c == 'm' {
                in_esc = false;
            }
            continue;
        }
        if c == '\x1b' {
            in_esc = true;
            continue;
        }
        n += 1;
    }
    n
}

/// Terminal width via `stty size`; falls back to 80 columns. Cheap
/// enough for one-shot `--docs` invocations.
fn term_width() -> usize {
    if let Ok((_, cols)) = terminal_size() {
        if cols > 0 {
            return cols as usize;
        }
    }
    80
}

fn terminal_size() -> Result<(u16, u16), std::io::Error> {
    use std::process::Command;
    let out = Command::new("stty").arg("size").output()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut parts = s.split_whitespace();
    let rows: u16 = parts
        .next()
        .and_then(|x| x.parse().ok())
        .ok_or_else(|| std::io::Error::other("no rows from stty"))?;
    let cols: u16 = parts
        .next()
        .and_then(|x| x.parse().ok())
        .ok_or_else(|| std::io::Error::other("no cols from stty"))?;
    Ok((rows, cols))
}

/// `zshrs --gen-docs [PATH] [--out DIR]` — walk PATH (default `.`),
/// find every `.zsh` / `.sh` / `.bash` / `.ksh` / dotfile source, and
/// emit per-file Markdown module docs under `--out DIR` (default
/// `docs/`). Output layout mirrors source layout: `lib/foo.zsh` →
/// `docs/lib/foo.md`. Also writes an `index.md` summary.
///
/// Mirrors `stryke gen-docs` for the zshrs CLI. Use this to ship
/// human-readable reference docs alongside your shell scripts —
/// extracts `##` doc-comments paired with the function decl below.
fn run_gen_docs_subcommand(args: &[&str]) -> i32 {
    if args.first().copied() == Some("-h") || args.first().copied() == Some("--help") {
        println!("usage: zshrs --gen-docs [PATH] [--out DIR]");
        println!();
        println!("Walk PATH (default `.`) for `.zsh` / `.sh` / `.bash` / `.ksh`");
        println!("/ rc-dotfile sources and emit Markdown module docs for each.");
        println!("Output goes under --out DIR (default `docs/`), mirroring the");
        println!("source layout.");
        println!();
        println!("Doc-comment convention: contiguous `##` lines IMMEDIATELY above");
        println!("a `function NAME` / `NAME()` declaration become that function's");
        println!("documentation. A `##` block at the top of the file becomes the");
        println!("module header.");
        return 0;
    }

    let mut path: Option<String> = None;
    let mut out_dir: String = "docs".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--out" | "-o" => {
                if i + 1 >= args.len() {
                    eprintln!("zshrs --gen-docs: --out requires a directory argument");
                    return 2;
                }
                out_dir = args[i + 1].to_string();
                i += 2;
            }
            other if !other.starts_with('-') && path.is_none() => {
                path = Some(other.to_string());
                i += 1;
            }
            other => {
                eprintln!("zshrs --gen-docs: unexpected argument: {other}");
                return 2;
            }
        }
    }
    let root = std::path::PathBuf::from(path.unwrap_or_else(|| ".".to_string()));
    if !root.exists() {
        eprintln!("zshrs --gen-docs: path does not exist: {}", root.display());
        return 1;
    }
    let mut sources: Vec<std::path::PathBuf> = Vec::new();
    zsh::gen_docs::collect_doc_sources(&root, &mut sources);
    sources.sort();
    if sources.is_empty() {
        eprintln!(
            "zshrs --gen-docs: no shell sources found under {}",
            root.display()
        );
        return 1;
    }
    let out_root = std::path::PathBuf::from(&out_dir);
    if let Err(e) = std::fs::create_dir_all(&out_root) {
        eprintln!(
            "zshrs --gen-docs: cannot create output dir {}: {}",
            out_root.display(),
            e
        );
        return 1;
    }
    let mut index: Vec<(String, String)> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut used_paths: std::collections::HashSet<std::path::PathBuf> = Default::default();
    for src in &sources {
        let source = match std::fs::read_to_string(src) {
            Ok(s) => s,
            Err(e) => {
                errors.push(format!("{}: {}", src.display(), e));
                continue;
            }
        };
        let md = zsh::gen_docs::generate_markdown(&src.to_string_lossy(), &source);
        let rel = src.strip_prefix(&root).unwrap_or(src);
        // Output path preserves the SOURCE extension as part of the
        // stem to dedupe collisions when the source tree has multiple
        // shells with the same basename (`foo.zsh` vs `foo.sh` vs
        // `foo.bash`). Without this each one overwrites the next and
        // the index lists duplicates pointing at the last writer.
        let stem = rel.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let ext = rel.extension().and_then(|s| s.to_str()).unwrap_or("");
        let renamed = if ext.is_empty() {
            format!("{}.md", stem)
        } else {
            format!("{}.{}.md", stem, ext)
        };
        let mut out_path = match rel.parent() {
            Some(p) if !p.as_os_str().is_empty() => out_root.join(p).join(renamed),
            _ => out_root.join(renamed),
        };
        // Last-resort uniqueness: append `-N` if two distinct sources
        // somehow still hash to the same output path.
        let mut n = 1;
        while used_paths.contains(&out_path) {
            n += 1;
            let stem2 = format!("{}.{}-{}", stem, ext, n);
            out_path = match rel.parent() {
                Some(p) if !p.as_os_str().is_empty() => {
                    out_root.join(p).join(format!("{}.md", stem2))
                }
                _ => out_root.join(format!("{}.md", stem2)),
            };
        }
        used_paths.insert(out_path.clone());
        // Discard the original `.md` redirect — we already built the
        // final path above. (Variable retained as type-anchor below.)
        let _ = ();
        if let Some(parent) = out_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                errors.push(format!("{}: mkdir failed: {}", parent.display(), e));
                continue;
            }
        }
        if let Err(e) = std::fs::write(&out_path, &md) {
            errors.push(format!("{}: write failed: {}", out_path.display(), e));
            continue;
        }
        let title = std::path::Path::new(rel.as_os_str())
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("(unknown)")
            .to_string();
        index.push((title, out_path.display().to_string()));
    }
    // index.md
    let mut idx = String::new();
    idx.push_str("# Module Index\n\n");
    idx.push_str(&format!("{} module(s) generated.\n\n", index.len()));
    for (title, path) in &index {
        idx.push_str(&format!(
            "- [{}]({})\n",
            title,
            std::path::Path::new(path)
                .strip_prefix(&out_root)
                .unwrap_or(std::path::Path::new(path))
                .display()
        ));
    }
    if let Err(e) = std::fs::write(out_root.join("index.md"), &idx) {
        eprintln!("zshrs --gen-docs: index write failed: {}", e);
    }
    eprintln!(
        "zshrs --gen-docs: wrote {} module(s) under {}",
        index.len(),
        out_root.display()
    );
    if !errors.is_empty() {
        eprintln!("zshrs --gen-docs: {} error(s):", errors.len());
        for e in &errors {
            eprintln!("  {}", e);
        }
        return 1;
    }
    0
}

fn print_help() {
    println!(
        r#"Usage: zsh [<options>] [<argument> ...]

Special options:
  --help       show this message, then exit
  --version    show zsh version number, then exit
  --doctor     full diagnostic report of shell health, caches, and performance

Parser-pipeline dumpers (FILE, or `-` for stdin; output goes to stdout):
  --dump-tokens   FILE   one TOKNAME<tab>TOKSTR line per lexer token
  --dump-ast      FILE   parser AST as canonical S-expression
  --dump-wordcode FILE   wordcode emitter output (EPROG / WORDS / WC[i] / STRS)
  --dump-zwc      ZWCFILE [FN]   inspect compiled .zwc cache (list or one fn)

VM debugging (stdout; does not suppress execution):
  --disasm       print fusevm opcodes for each compiled unit before VM run

Editor integration (served by zshrs itself, consumed by editors/intellij
and other LSP/DAP clients — Helix, Neovim, VS Code, …):
  --lsp                  LSP server on stdio (completion / hover / definition
                         / references / rename / documentSymbol / foldingRange
                         / semanticTokens / formatting / diagnostics)
  --dap [HOST:PORT]      DAP debug adapter. With HOST:PORT, connects back to the
                         IDE's listener over TCP (JetBrains). Without, serves DAP
                         over stdio (for executable-spawned clients, e.g. VS Code)
  --dump-reflection      emit the JSON consumed by the IntelliJ "zshrs"
                         reflection tool window (builtins / keywords / options
                         / special_vars, each tagged by category)
  --dump-plugins         emit the JSON consumed by the IntelliJ External
                         Libraries view: every sourced plugin grouped by
                         manager (zinit / oh-my-zsh / prezto / antidote /
                         antigen / zplug / zsh-more-completions / zpwr / loose)
  --docs NAME            render the hover card the LSP would return for NAME
                         (used by the IntelliJ tool window's docs popup)

Parity modes (caches OFF, daemon OFF — match the named reference shell
byte-for-byte; every `source` re-runs the file fresh, every echo re-fires):
  --zsh        identical-behaviour drop-in for /bin/zsh (compat-test entrypoint)
  --bash       identical-behaviour drop-in for /bin/bash
  --ksh        identical-behaviour drop-in for /bin/ksh (ksh-93)
  --mksh       identical-behaviour drop-in for mksh (MirBSD ksh; ksh base)
  --pdksh      identical-behaviour drop-in for pdksh (Public Domain ksh; ksh base)
  --sh         identical-behaviour drop-in for /bin/sh / POSIX (alias of --posix)
  --dash       identical-behaviour drop-in for /bin/dash (strict POSIX subset)
  --ash        identical-behaviour drop-in for ash (Almquist; alias of --dash)
  --csh        identical-behaviour drop-in for /bin/csh
  --posix      identical-behaviour drop-in for /bin/sh (Bourne)
  --emulate MODE  alias for --MODE (zsh-compat: `emulate zsh` etc.)
  --zsh-compat alias of --zsh (legacy spelling)

Argv[0] inference: invoking the binary as `ksh` / `sh` / `csh` / `bash`
(via symlink or hardlink) selects the matching mode automatically, matching
C zsh's behaviour at Src/init.c:1869+. Use the explicit `--MODE` flag to
override.

Default mode (no flag) is full zshrs: rkyv script_cache + plugin_cache
+ daemon enabled. Use the parity flags for compat testing or when caching
behavior is unwanted.

Standard zsh options:
  -b           end option processing, like --
  -c           take first argument as a command to execute
  -f           equivalent to --no-rcs (don't source startup files)
  -i           force interactive mode
  -l           force login shell mode
  -s           read commands from stdin
  -o OPTION    set an option by name (see below)
  -v           verbose (equivalent to --verbose)
  -x           xtrace (equivalent to --xtrace)

Normal options are named.  An option may be turned on by
`-o OPTION', `--OPTION', `+o no_OPTION' or `+-no-OPTION'.  An
option may be turned off by `-o no_OPTION', `--no-OPTION',
`+o OPTION' or `+-OPTION'.  Options are listed below only in
`--OPTION' or `--no-OPTION' form.

Named options:
  --aliases
  --aliasfuncdef
  --allexport
  --alwayslastprompt
  --alwaystoend
  --appendcreate
  --appendhistory
  --autocd
  --autocontinue
  --autolist
  --automenu
  --autonamedirs
  --autoparamkeys
  --autoparamslash
  --autopushd
  --autoremoveslash
  --autoresume
  --badpattern
  --banghist
  --bareglobqual
  --bashautolist
  --bashrematch
  --beep
  --bgnice
  --braceccl
  --bsdecho
  --caseglob
  --casematch
  --casepaths
  --cbases
  --cdablevars
  --cdsilent
  --chasedots
  --chaselinks
  --checkjobs
  --checkrunningjobs
  --clobber
  --clobberempty
  --combiningchars
  --completealiases
  --completeinword
  --continueonerror
  --correct
  --correctall
  --cprecedences
  --cshjunkiehistory
  --cshjunkieloops
  --cshjunkiequotes
  --cshnullcmd
  --cshnullglob
  --debugbeforecmd
  --dvorak
  --emacs
  --equals
  --errexit
  --errreturn
  --evallineno
  --exec
  --extendedglob
  --extendedhistory
  --flowcontrol
  --forcefloat
  --functionargzero
  --glob
  --globalexport
  --globalrcs
  --globassign
  --globcomplete
  --globdots
  --globstarshort
  --globsubst
  --hashcmds
  --hashdirs
  --hashexecutablesonly
  --hashlistall
  --histallowclobber
  --histbeep
  --histexpiredupsfirst
  --histfcntllock
  --histfindnodups
  --histignorealldups
  --histignoredups
  --histignorespace
  --histlexwords
  --histnofunctions
  --histnostore
  --histreduceblanks
  --histsavebycopy
  --histsavenodups
  --histsubstpattern
  --histverify
  --hup
  --ignorebraces
  --ignoreclosebraces
  --ignoreeof
  --incappendhistory
  --incappendhistorytime
  --interactive
  --interactivecomments
  --ksharrays
  --kshautoload
  --kshglob
  --kshoptionprint
  --kshtypeset
  --kshzerosubscript
  --listambiguous
  --listbeep
  --listpacked
  --listrowsfirst
  --listtypes
  --localloops
  --localoptions
  --localpatterns
  --localtraps
  --login
  --longlistjobs
  --magicequalsubst
  --mailwarning
  --markdirs
  --menucomplete
  --monitor
  --multibyte
  --multifuncdef
  --multios
  --nomatch
  --notify
  --nullglob
  --numericglobsort
  --octalzeroes
  --overstrike
  --pathdirs
  --pathscript
  --pipefail
  --posixaliases
  --posixargzero
  --posixbuiltins
  --posixcd
  --posixidentifiers
  --posixjobs
  --posixstrings
  --posixtraps
  --printeightbit
  --printexitvalue
  --privileged
  --promptbang
  --promptcr
  --promptpercent
  --promptsp
  --promptsubst
  --pushdignoredups
  --pushdminus
  --pushdsilent
  --pushdtohome
  --rcexpandparam
  --rcquotes
  --rcs
  --recexact
  --rematchpcre
  --restricted
  --rmstarsilent
  --rmstarwait
  --sharehistory
  --shfileexpansion
  --shglob
  --shinstdin
  --shnullcmd
  --shoptionletters
  --shortloops
  --shortrepeat
  --shwordsplit
  --singlecommand
  --singlelinezle
  --sourcetrace
  --sunkeyboardhack
  --transientrprompt
  --trapsasync
  --typesetsilent
  --typesettounset
  --unset
  --verbose
  --vi
  --warncreateglobal
  --warnnestedvar
  --xtrace
  --zle

Option aliases:
  --braceexpand            equivalent to --no-ignorebraces
  --dotglob                equivalent to --globdots
  --hashall                equivalent to --hashcmds
  --histappend             equivalent to --appendcreate
  --histexpand             equivalent to --badpattern
  --log                    equivalent to --no-histnofunctions
  --mailwarn               equivalent to --mailwarning
  --onecmd                 equivalent to --singlecommand
  --physical               equivalent to --cdsilent
  --promptvars             equivalent to --promptsubst
  --stdin                  equivalent to --shinstdin
  --trackall               equivalent to --hashcmds

Option letters:
  -0    equivalent to --completeinword
  -1    equivalent to --printexitvalue
  -2    equivalent to --no-autoresume
  -3    equivalent to --no-nomatch
  -4    equivalent to --globdots
  -5    equivalent to --notify
  -6    equivalent to --beep
  -7    equivalent to --ignoreeof
  -8    equivalent to --markdirs
  -9    equivalent to --autocontinue
  -B    equivalent to --no-bashrematch
  -C    equivalent to --no-checkjobs
  -D    equivalent to --pushdtohome
  -E    equivalent to --pushdsilent
  -F    equivalent to --no-glob
  -G    equivalent to --nullglob
  -H    equivalent to --rmstarsilent
  -I    equivalent to --ignorebraces
  -J    equivalent to --appendhistory
  -K    equivalent to --no-badpattern
  -L    equivalent to --sunkeyboardhack
  -M    equivalent to --singlelinezle
  -N    equivalent to --autoparamslash
  -O    equivalent to --continueonerror
  -P    equivalent to --rcexpandparam
  -Q    equivalent to --pathdirs
  -R    equivalent to --longlistjobs
  -S    equivalent to --recexact
  -T    equivalent to --cbases
  -U    equivalent to --mailwarning
  -V    equivalent to --no-promptcr
  -W    equivalent to --autoremoveslash
  -X    equivalent to --listtypes
  -Y    equivalent to --menucomplete
  -Z    equivalent to --zle
  -a    equivalent to --allexport
  -d    equivalent to --no-globalrcs
  -e    equivalent to --errexit
  -f    equivalent to --no-rcs
  -g    equivalent to --histignorespace
  -h    equivalent to --histignoredups
  -i    equivalent to --interactive
  -k    equivalent to --interactivecomments
  -l    equivalent to --login
  -m    equivalent to --monitor
  -n    equivalent to --no-exec
  -p    equivalent to --privileged
  -r    equivalent to --restricted
  -s    equivalent to --shinstdin
  -t    equivalent to --singlecommand
  -u    equivalent to --no-unset
  -v    equivalent to --verbose
  -w    equivalent to --cdsilent
  -x    equivalent to --xtrace
  -y    equivalent to --shwordsplit
"#
    );
}

/// Shell mode: zshrs (default), --zsh (zsh drop-in), --bash (bash drop-in),
/// --posix (POSIX sh / Bourne strict).
///
/// Each mode is a parity-target: the visible behavior should match the named
/// shell byte-for-byte where the script under test only uses features common
/// to that shell. zshrs's caches, daemon, and zshrs-exclusive builtins are
/// disabled in every mode except `Zshrs` so observable side-effects (stdout,
/// signal handlers, file I/O) re-fire on every invocation just like the
/// reference shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMode {
    /// Full zshrs — all features, --doctor, plugin cache UI, exclusive builtins.
    Zshrs,
    /// zsh drop-in — identical to C zsh (Src/builtin.c bin_dot, no caches,
    /// no daemon, no plugin replay). Used for compat-testing zshrs against
    /// /bin/zsh with `zshrs --zsh script.zsh`.
    Zsh,
    /// bash drop-in — bash 5.x semantics. zsh extensions disabled; bash-only
    /// features (e.g. `BASH_VERSION`, `[[ =~ ]]` BASH_REMATCH, `mapfile`,
    /// `readarray`, `${arr[@]: ... }` slice rules) preferred. No caches,
    /// no daemon. Used for `zshrs --bash script.sh` parity tests against
    /// /bin/bash.
    Bash,
    /// ksh-93 drop-in — Korn-shell semantics. Applies the same option
    /// presets that `emulate ksh` would (`ksharrays`, `kshglob`,
    /// `shwordsplit`, `posixbuiltins`, `kshoptionprint`, `promptbang`,
    /// etc.). zsh-only extensions disabled. No caches, no daemon. Used
    /// for `zshrs --ksh script.ksh` parity tests against /bin/ksh.
    Ksh,
    /// POSIX sh / Bourne strict — only POSIX builtins, no zsh / bash
    /// extensions, no arrays, no `[[`, no extended globbing, no SQLite
    /// caches, no worker pool, no daemon. Used for parity tests against
    /// `/bin/sh` (Bourne).
    Posix,
    /// dash (Debian Almquist Shell) strict — `sh` emulation PLUS the
    /// strict-POSIX syntactic subset that makes zshrs reject exactly what
    /// `/bin/dash` rejects: `$'...'`, `<<<`, `+=`, `name=(...)` arrays,
    /// the `[[ ]]` reserved word, arith `**` / `,`, and non-XSI echo.
    /// Drives `emulate("dash")` → `EMULATE_SH` + `DASH_STRICT`. Used for
    /// parity tests against `/bin/dash` with `zshrs --dash script.sh`.
    Dash,
}

static mut SHELL_MODE: ShellMode = ShellMode::Zshrs;

/// Global log file path for zshrs background operations (compinit, etc.)
pub fn zshrs_log_path() -> PathBuf {
    // Single source of truth: zsh::log::log_path() honors $ZSHRS_HOME
    // and falls back to ~/.zshrs/zshrs.log.
    zsh::log::log_path()
}
/// `shell_mode` — see implementation.
pub fn shell_mode() -> ShellMode {
    unsafe { SHELL_MODE }
}
/// `is_zsh_mode` — see implementation.
pub fn is_zsh_mode() -> bool {
    matches!(shell_mode(), ShellMode::Zsh)
}
/// `is_bash_mode` — see implementation.
pub fn is_bash_mode() -> bool {
    matches!(shell_mode(), ShellMode::Bash)
}
/// `is_ksh_mode` — see implementation.
pub fn is_ksh_mode() -> bool {
    matches!(shell_mode(), ShellMode::Ksh)
}
/// `is_posix_mode` — see implementation.
pub fn is_posix_mode() -> bool {
    matches!(shell_mode(), ShellMode::Posix)
}
/// `is_dash_mode` — true in strict-dash parity mode (`zshrs --dash`).
pub fn is_dash_mode() -> bool {
    matches!(shell_mode(), ShellMode::Dash)
}
/// `is_zshrs_mode` — see implementation.
pub fn is_zshrs_mode() -> bool {
    matches!(shell_mode(), ShellMode::Zshrs)
}

/// Legacy compat shim — maps to --zsh mode
pub fn is_zsh_compat() -> bool {
    is_zsh_mode()
}

/// Recovery of the process-entry environment.
///
/// zsh reads `extern char **environ` in `createparamtable`
/// (c:Src/params.c:893) and nothing in the zsh process mutates it before
/// that point, so the C shell always imports exactly what `execve` passed.
/// A Rust binary on macOS does not get that for free: dyld runs the
/// initializers of every linked library before `main`, and CoreFoundation's
/// initializer calls `setenv("__CF_USER_TEXT_ENCODING", ...)`. CF is linked
/// transitively (chrono -> iana-time-zone -> core-foundation-sys, and
/// notify -> fsevent-sys -> CoreServices -> CoreFoundation), neither of
/// which can be dropped: `cron` pins `chrono` with `features = ["clock"]`
/// and `notify-debouncer-mini` pins `notify` with default features, so
/// Cargo's feature unification re-enables both no matter what this crate
/// requests. The result is one phantom parameter in every environment- or
/// parameter-enumerating completion.
///
/// The fix uses the one copy of the environment that CF's `setenv` cannot
/// reach. Mach-O `__DATA,__mod_init_func` entries are invoked by dyld with
/// the C `main` signature plus extras — `(argc, argv, envp, apple, vars)` —
/// and that `envp` is the array the kernel wrote onto the process stack at
/// `execve` time, not the `environ` pointer. When `setenv` has to grow the
/// array (which is what adding a new name always requires) it allocates a
/// fresh one on the heap and repoints `environ` at it, leaving the stack
/// array untouched. So a variable that the parent never passed is by
/// construction absent here, while every variable the parent did pass is
/// still present.
///
/// Verified with a standalone probe binary linking chrono:
/// `env -i A=1 B=2 C=3 ./probe` reported `orig_n=3 live_n=4`, the single
/// extra live name being `__CF_USER_TEXT_ENCODING`; with a 409-variable
/// inherited environment it reported `orig_n=409 live_n=409` with no name
/// added, dropped, or value-changed. Nothing pre-`main` calls `unsetenv`,
/// which is the only operation that could shift entries out of the stack
/// array; if that ever changed, the affected name would be missing from
/// the live environment too, so the shell cannot end up worse off.
///
/// Value-only caveat: when the parent *did* export
/// `__CF_USER_TEXT_ENCODING`, `setenv` overwrites the array slot in place
/// (the array is not grown), so the recovered value is CF's rewritten one
/// rather than the inherited one. That divergence is pre-existing and
/// already pinned by `export_minus_p_full_dump` in
/// `tests/parity/zsh_compat_parity_gaps.rs`; only the phantom-name case is
/// addressed here.
#[cfg(target_os = "macos")]
mod initial_env {
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_int};
    use std::sync::atomic::{AtomicPtr, Ordering};

    /// The stack `envp` dyld handed us, or null if the initializer never
    /// ran (e.g. a future linker drops the section). Null means "fall
    /// back to the live environment"; a non-null pointer to an
    /// immediately-NULL-terminated array is a genuinely empty
    /// environment (`env -i`) and must be honoured as such.
    static ENTRY_ENVP: AtomicPtr<*const c_char> = AtomicPtr::new(std::ptr::null_mut());

    /// dyld passes `(argc, argv, envp, apple, vars)`; the trailing
    /// arguments are ignored, which is ABI-safe in both the SysV and
    /// AAPCS64 C calling conventions.
    extern "C" fn capture_entry_envp(
        _argc: c_int,
        _argv: *const *const c_char,
        envp: *mut *const c_char,
        _apple: *const *const c_char,
    ) {
        ENTRY_ENVP.store(envp, Ordering::SeqCst);
    }

    #[link_section = "__DATA,__mod_init_func"]
    #[used]
    static MOD_INIT_FUNC: [extern "C" fn(
        c_int,
        *const *const c_char,
        *mut *const c_char,
        *const *const c_char,
    ); 1] = [capture_entry_envp];

    /// Walk the captured array. Runs from `main`, after every dyld
    /// initializer has finished, so the array is stable. Values are
    /// decoded lossily rather than panicking the way `std::env::vars()`
    /// does on non-UTF-8 — a shell must not die on a hostile environment.
    pub fn snapshot() -> Option<Vec<(String, String)>> {
        let envp = ENTRY_ENVP.load(Ordering::SeqCst);
        if envp.is_null() {
            return None;
        }
        let mut out = Vec::new();
        // SAFETY: `envp` is the NULL-terminated array dyld passed at
        // process entry; it lives on the process stack for the lifetime
        // of the process and libc never frees or shortens it.
        unsafe {
            let mut i = 0isize;
            loop {
                let entry = *envp.offset(i);
                if entry.is_null() {
                    break;
                }
                let s = String::from_utf8_lossy(CStr::from_ptr(entry).to_bytes());
                if let Some((name, value)) = s.split_once('=') {
                    out.push((name.to_string(), value.to_string()));
                }
                i += 1;
            }
        }
        Some(out)
    }
}

/// Delete from the live process environment every name that was injected
/// after `execve` and before `main` — i.e. every live name absent from the
/// process-entry `envp`. In practice that set is exactly
/// `{__CF_USER_TEXT_ENCODING}` (empty when the parent already exported it).
///
/// Fixing the parameter-table import alone is not enough. `getsparam` falls
/// back to the live environment on a table miss
/// (`src/ported/params.rs:5544-5547`), as do `zgetenv`
/// (`src/ported/params.rs:11228`) and `findenv`
/// (`src/ported/params.rs:11212`), so an injected variable stays visible to
/// `${+name}` and friends however clean the table is. It would also be
/// exported to every child process, which zsh's children never see. Pruning
/// the live environment closes all of those at once and leaves the process
/// environment byte-identical to what zsh would be running with.
///
/// Only names the parent provably did not pass are removed, so no
/// inherited variable can be lost. CoreFoundation reads
/// `__CF_USER_TEXT_ENCODING` once from its own initializer and only
/// re-publishes it for child processes, so removing it after `main` starts
/// does not disturb the frameworks that are already initialised.
#[cfg(target_os = "macos")]
fn prune_preinit_env_injections(entry: &[(String, String)]) {
    use std::collections::HashSet;
    let entry_names: HashSet<&str> = entry.iter().map(|(k, _)| k.as_str()).collect();
    let injected: Vec<String> = std::env::vars()
        .map(|(k, _)| k)
        .filter(|k| !entry_names.contains(k.as_str()))
        .collect();
    for name in injected {
        tracing::debug!(
            "pruning pre-main environment injection: {} (absent from process-entry envp)",
            name
        );
        std::env::remove_var(name);
    }
}

/// Non-macOS: no pre-`main` initializer rewrites the environment, so the
/// live one already is the process-entry one and no recovery is needed.
#[cfg(not(target_os = "macos"))]
mod initial_env {
    pub fn snapshot() -> Option<Vec<(String, String)>> {
        None
    }
}

fn main() {
    // c:Src/params.c:893 createparamtable reads `environ` exactly as
    // it was at process entry. Snapshot it as the first statement so
    // nothing later in shell init (setenv from builtins, lazy crate
    // init) skews the import.
    //
    // On macOS `std::env::vars()` is NOT the process-entry environment:
    // zshrs links CoreFoundation (chrono -> iana-time-zone ->
    // core-foundation-sys) and CoreServices (notify -> fsevent-sys), and
    // CF's dyld initializer runs before `main` and `setenv`s
    // __CF_USER_TEXT_ENCODING. zsh links neither, so it imports an
    // environment without that variable. `initial_env::snapshot()`
    // recovers the real one from the stack `envp` dyld hands to a
    // `__mod_init_func` entry (see the module below); it returns None on
    // non-macOS and on any capture failure, in which case the live
    // environment is used exactly as before. The kernel's exec-image copy
    // (sysctl KERN_PROCARGS2) was tried earlier and REJECTED: it silently
    // truncates large environments (tail vars vanish).
    let entry_env = initial_env::snapshot();
    #[cfg(target_os = "macos")]
    if let Some(entry) = entry_env.as_deref() {
        prune_preinit_env_injections(entry);
    }
    let _ = zsh::ported::params::environ
        .set(entry_env.unwrap_or_else(|| std::env::vars().collect()));
    // Restore default SIGPIPE behavior before anything writes to
    // stdout/stderr. Rust runtime installs SIG_IGN on SIGPIPE in
    // some Linux builds and ignores it on macOS — either way,
    // writes to a closed pipe yield an EPIPE error that bubbles
    // up and panics the println!/writeln! callers in any builtin
    // emitting multi-line output (\`set -o\`, \`setopt\`,
    // \`functions\`, etc.).
    //
    // zsh-the-program writes are bare write() syscalls with no
    // EPIPE recovery, so the C process dies silently with status
    // 141 on SIGPIPE. Match that: install SIG_DFL so we terminate
    // on broken pipe instead of panicking. Test: \`set -o | head -3\`
    // exited 1 with a panic stack trace; with SIG_DFL it exits 141
    // and emits only the head'd lines.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    // The shell runs on the main thread (keeping all signal/job-control
    // semantics intact). Deep function recursion needs a large stack —
    // each zshrs call frame is heavy (fusevm executor state, closures,
    // parse buffers) and real configs (p10k/zinit precmd hooks) nest
    // deeply per call, so on the default 8 MB main-thread stack a deep
    // (or runaway) recursion overflowed and SEGFAULTed before the
    // FUNCNEST guard could fire. Rather than move the shell onto a
    // spawned thread (which breaks async-signal/trap delivery — signals
    // race to worker threads), the main-thread stack itself is enlarged
    // at link time via `-stack_size` (see build.rs). That lets recursion
    // up to FUNCNEST (default 500) complete and lets the guard turn true
    // runaways into a zsh-matching error instead of a crash.
    zshrs_main();
}

/// Main entry point — extracted so the fat binary can call it after
/// registering the stryke handler.
pub fn zshrs_main() {
    // Initialize logging first — everything after this can use tracing macros.
    let startup_t0 = Instant::now();

    // Make sure ~/.zshrs exists with the default config files. Every
    // binary (zshrs / zshrs-daemon / zshrs-recorder / zd) does this,
    // so whoever runs first gets the user a fully-populated
    // `~/.zshrs/` tree without manual intervention. Idempotent —
    // never overwrites a user-edited file.
    #[cfg(feature = "daemon")]
    if let Ok(paths) = zshrs_daemon::paths::CachePaths::resolve() {
        let _ = paths.ensure_dirs();
        let _ = paths.ensure_default_configs();
    }

    // Hand the daemon-crate `zsync up --all` builtin a snapshot
    // function it can call into the executor with. The shell crate
    // owns ShellExecutor; the daemon crate (where the zsync builtin
    // lives) doesn't link against it — this trampoline is the
    // bridge. Idempotent — only the first registration sticks.
    #[cfg(feature = "daemon")]
    zshrs_daemon::zsync_builtin::register_overlay_enumerator(
        zsh::overlay_snapshot::enumerate_all_overlays,
    );

    // Per-mode log file so the three server processes don't interleave
    // their tracing output in `zshrs.log`:
    //   * `zshrs --lsp` (spawned by IDE plugin) → `zshrs-lsp.log`
    //   * `zshrs --dap HOST:PORT` (spawned per debug session) → `zshrs-dap.log`
    //   * everything else (interactive shell, --version, --docs, …) → `zshrs.log`
    // The plugin side has its own `zshrs-plugin.log` (ZshrsDebugLog).
    // Daemon already routes through `zshrs-daemon.log` via daemon/log.rs.
    // OnceLock semantics mean only the first init() call wins — must
    // scan args BEFORE calling init so the right filename is picked.
    // Default level: info. Override with ZSHRS_LOG=debug or ZSHRS_LOG=trace.
    let log_name = {
        let raw_args: Vec<String> = env::args().collect();
        if raw_args.iter().any(|a| a == "--lsp") {
            "zshrs-lsp.log"
        } else if raw_args.iter().any(|a| a == "--dap") {
            "zshrs-dap.log"
        } else {
            "zshrs.log"
        }
    };
    zsh::log::init_named(log_name);

    // Single-shot daemon-presence probe. Honors `[daemon].enabled` in
    // ~/.config/zshrs/zshrs.toml (auto / off / require). After this,
    // call sites use `zsh::daemon_presence::is_present()` for an
    // O(1) atomic check before issuing IPC. Daemon absent = the
    // shell runs in vanilla mode (re-evaluate every config per launch
    // — "rebuilding your house every morning").
    let _ = zsh::daemon_presence::probe();

    // Arm the lineage engine when `[provenance] track_all` asks for it.
    // Before this call the engine is inert whatever the config says —
    // nothing has read it.
    zsh::provenance::init_from_config();

    // Pre-warm any per-process caches that depend on knowing the
    // current PID — pid lookup is cheap, but the call site here
    // exists so the call appears in the trace.
    let _ = std::process::id();
    let pid = std::process::id();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "?".to_string());
    let path_count = env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .count();
    let fpath_count = env::var("FPATH")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .count();
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    tracing::info!(
        pid,
        cwd = %cwd,
        path_dirs = path_count,
        fpath_dirs = fpath_count,
        cpus,
        "zshrs starting"
    );

    let args: Vec<String> = {
        // Expand POSIX-style clumped short options BEFORE any flag check.
        // `zshrs -fc 'pwd'` should behave identically to `zshrs -f -c 'pwd'`
        // — real zsh accepts both forms (verified: `zsh -fc 'pwd'` and
        // `zsh -cf 'pwd'` both run the command). Without expansion, our
        // dispatch's literal `args[1] == "-c"` check missed the clumped
        // form and the binary silently exited 0 with no command run,
        // which broke ztst harnesses and any caller copying zsh syntax.
        //
        // Rule: any arg matching `-[A-Za-z]{2,}` (single dash, 2+ alpha
        // chars, no `=`, not `--`) splits into individual `-X` tokens.
        // Long options keep the `--` prefix; numeric/symbolic clumps
        // are left alone. Argument-consuming flags like `-c` get their
        // value from the next argv slot after expansion, same as zsh.
        //
        // Exception: a clump containing `o` is left intact. c:528-529
        // (`if (!*++*argv) argv++;`) makes the REST of the word after
        // `o` the option NAME, so `-onullglob` is `-o nullglob` — not
        // nine separate letters. Splitting it produced `-o -n -u -l …`
        // and lost the option entirely; the option-word walk below
        // handles clumps natively via its own character loop.
        let raw: Vec<String> = env::args().collect();
        let mut out: Vec<String> = Vec::with_capacity(raw.len());
        for a in &raw {
            let bytes = a.as_bytes();
            let is_clumped = bytes.len() >= 3
                && bytes[0] == b'-'
                && bytes[1] != b'-'
                && bytes.iter().skip(1).all(|c| c.is_ascii_alphabetic())
                && !bytes.iter().skip(1).any(|c| *c == b'o');
            if is_clumped {
                for c in bytes.iter().skip(1) {
                    out.push(format!("-{}", *c as char));
                }
            } else {
                out.push(a.clone());
            }
        }
        out
    };

    zsh::fusevm_disasm::set_enabled(args.iter().any(|a| a == "--disasm"));

    // AOT trailer probe: if this binary was produced by `zbuild`, the last
    // 32 bytes contain a magic + length pair pointing at a zstd-compressed
    // payload of one-or-more shell scripts appended to the executable.
    // Detect, decode, and run each in input order under a single executor —
    // globals + functions defined by file N are visible to file N+1.
    // Without a trailer this is a no-op (all binaries get the same probe).
    if let Ok(self_exe) = env::current_exe() {
        if let Some(embedded) = zsh::aot::try_load_embedded(&self_exe) {
            // Remove our argv[0] (the binary path); positional args remain
            // for the entire bundle (every file sees the same $1..$N).
            let script_args: Vec<String> = args.iter().skip(1).cloned().collect();
            let mut executor = zsh::vm_helper::ShellExecutor::new();
            executor.set_pparams(script_args);
            let mut last_status = 0;
            for file in &embedded.0 {
                executor.set_scalar("0".to_string(), file.name.clone());
                last_status = match executor.execute_script(&file.source) {
                    Ok(s) => s,
                    Err(e) => {
                        if e != "__SILENCED__" {
                            eprintln!("zshrs: {}: {}", file.name, e);
                        }
                        std::process::exit(1);
                    }
                };
                // If a script called `exit N` (zsh's exit propagates via
                // the `returning` field in subshell-snapshot scope, but
                // here at top scope it terminates the process directly
                // through builtin_exit's process::exit). If we reach this
                // line the script ran to completion without calling exit;
                // continue to the next file.
                //
                // c:Src/init.c:1663 — each bundled file is source-like:
                // an errflag abort in file N must not poison file N+1's
                // first list. Clear the flag at the file boundary, keep
                // last_status (the aborted file's lastval).
                zsh::ported::utils::errflag.fetch_and(
                    !zsh::ported::zsh_h::ERRFLAG_ERROR,
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
            std::process::exit(last_status);
        }
    }

    // Argv[0] inference — matches C zsh's behaviour at
    // Src/init.c:1869+. Invoking via `ksh` / `sh` / `csh` / `bash`
    // symlink selects the mode before explicit flags are parsed.
    let argv0_basename: String = args
        .first()
        .map(|a| {
            let bare = a.trim_start_matches('-'); // strip login-shell `-` prefix
            std::path::Path::new(bare)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(bare)
                .to_string()
        })
        .unwrap_or_default();
    let argv0_inferred_mode: Option<ShellMode> = match argv0_basename.as_str() {
        "ksh" | "ksh93" | "mksh" | "pdksh" => Some(ShellMode::Ksh),
        "dash" | "ash" => Some(ShellMode::Dash),
        "sh" => Some(ShellMode::Posix),
        "bash" => Some(ShellMode::Bash),
        "zsh" | "zsh-5.9" => Some(ShellMode::Zsh),
        _ => None,
    };

    // Handle shell mode flags. Every parity mode (`--zsh`, `--bash`,
    // `--ksh`, `--sh`, `--csh`, `--posix`) force-disables zshrs's
    // caches and daemon by setting `ZSHRS_CACHE=0`. Explicit flags
    // override argv[0] inference.
    //
    // `--emulate MODE` form is zsh-compat (Src/init.c:443) — looks at
    // the next arg as the mode name; same final mapping.
    // The zsh-STYLE cross-emulation combos — `--sh --zsh`, `--ksh --zsh`, … —
    // mean "the X OPTION deltas applied ON TOP OF zsh's own parser/semantics",
    // i.e. exactly what `emulate X` does INSIDE a real zsh (lenient brace parse,
    // zsh base-digit case, `let` auto-typing, …), NOT the real-shell drop-in.
    // So SHELL_MODE stays Zsh (keeping the zsh parser); `zsh_style_emu` names
    // the option deltas to layer on. Only fires when `--zsh` accompanies a
    // sub-mode flag — a bare `--sh` is still the strict real-shell drop-in.
    let has_zsh = args.iter().any(|a| a == "--zsh" || a == "--zsh-compat");
    let zsh_style_emu: Option<&str> = if has_zsh {
        if args.iter().any(|a| a == "--dash" || a == "--ash") {
            Some("dash")
        } else if args
            .iter()
            // bash shares the sh emulation base, so it selects the same mode.
            .any(|a| a == "--posix" || a == "--sh" || a == "--bash")
        {
            Some("sh")
        } else if args
            .iter()
            .any(|a| a == "--ksh" || a == "--mksh" || a == "--pdksh")
        {
            Some("ksh")
        } else {
            None
        }
    } else {
        None
    };
    let explicit_mode: Option<ShellMode> = if zsh_style_emu.is_some() {
        // zsh-STYLE: zsh parser/semantics; the option deltas apply via emu_name.
        Some(ShellMode::Zsh)
    } else if args.iter().any(|a| a == "--dash" || a == "--ash") {
        // ash and dash are the same Almquist strict-POSIX shell family.
        Some(ShellMode::Dash)
    } else if args.iter().any(|a| a == "--posix" || a == "--sh") {
        Some(ShellMode::Posix)
    } else if args.iter().any(|a| a == "--bash") {
        Some(ShellMode::Bash)
    } else if args
        .iter()
        .any(|a| a == "--ksh" || a == "--mksh" || a == "--pdksh")
    {
        // mksh (MirBSD Korn shell) and pdksh (Public Domain Korn shell)
        // use the same ksh emulation base.
        Some(ShellMode::Ksh)
    } else if args.iter().any(|a| a == "--csh") {
        // csh emulation routes to Zsh mode (zshrs has no separate csh
        // bucket; `emulate csh` flips the canonical option deltas via
        // `crate::ported::options::emulate("csh", true)` below).
        Some(ShellMode::Zsh)
    } else if args.iter().any(|a| a == "--zsh" || a == "--zsh-compat") {
        Some(ShellMode::Zsh)
    } else if let Some(emu_idx) = args.iter().position(|a| a == "--emulate") {
        // `--emulate MODE` — consume the next arg as the mode name.
        match args.get(emu_idx + 1).map(|s| s.as_str()) {
            Some("ksh" | "mksh" | "pdksh") => Some(ShellMode::Ksh),
            Some("dash" | "ash") => Some(ShellMode::Dash),
            Some("sh" | "posix") => Some(ShellMode::Posix),
            Some("bash") => Some(ShellMode::Bash),
            Some("csh" | "zsh") => Some(ShellMode::Zsh),
            _ => None,
        }
    } else {
        None
    };
    let selected_mode = explicit_mode.or(argv0_inferred_mode);
    let parity_mode_selected = if let Some(mode) = selected_mode {
        unsafe {
            SHELL_MODE = mode;
        }
        // Mirror the binary-side ShellMode into the library-side
        // IS_ZSH_MODE atomic so bridge / dispatch sites in lib that
        // need to gate bash-compat-vs-zsh behavior can read it
        // without a binary-to-library reach-in. Bugs
        // #475/#504/#555 in docs/BUGS.md.
        zsh::IS_ZSH_MODE.store(
            matches!(mode, ShellMode::Zsh),
            std::sync::atomic::Ordering::Relaxed,
        );
        true
    } else {
        false
    };

    // Apply the canonical emulation option deltas via the ported
    // `crate::ported::options::emulate` (port of Src/options.c:533).
    // This is what `emulate ksh` / `emulate sh` / `emulate csh`
    // builtin does at runtime; calling it here from the bin entry
    // mirrors the C source's parseopts_setemulate (Src/init.c:348)
    // path that runs during shell init for `--ksh` etc. The Zsh /
    // Zshrs modes still go through this to set the canonical
    // EMULATE_ZSH bitmap (idempotent — that's the default).
    let emu_name = if let Some(sub) = zsh_style_emu {
        // zsh-STYLE (`--X --zsh`): SHELL_MODE is Zsh, but the sub-mode's option
        // deltas still layer on — this is what `emulate X` inside a real zsh does.
        sub
    } else {
        match shell_mode() {
            ShellMode::Ksh => "ksh",
            ShellMode::Dash => "dash", // sh emulation + DASH_STRICT (see options::emulate)
            ShellMode::Posix => "sh",
            ShellMode::Bash => "sh", // bash ≈ sh emulation; bash-specific bits flagged via is_bash_mode()
            ShellMode::Zsh | ShellMode::Zshrs => "zsh",
        }
    };
    if argv0_basename == "csh" || args.iter().any(|a| a == "--csh") {
        zsh::ported::options::emulate("csh", true);
    } else {
        zsh::ported::options::emulate(emu_name, true);
    }

    // Real-shell-faithful toggle: a bare POSIX-family drop-in
    // (`--sh`/`--ksh`/`--dash`) makes zshrs match the ACTUAL shell rather
    // than zsh's approximation of it (e.g. trailing-empty-field splitting).
    // Adding `--zsh` (`zshrs --sh --zsh`) selects zsh-style emulation
    // instead by clearing the flag. See extensions::dash_mode.
    let posix_family = matches!(
        shell_mode(),
        ShellMode::Posix | ShellMode::Ksh | ShellMode::Dash | ShellMode::Bash
    );
    let zsh_style_requested = args.iter().any(|a| a == "--zsh" || a == "--zsh-compat");
    zsh::extensions::dash_mode::set_posix_faithful(posix_family && !zsh_style_requested);

    // bash-specific option deltas on top of the shared `sh` emulation base.
    // bash is a SUPERSET of POSIX sh: unlike `emulate sh` (which sets
    // IGNOREBRACES), bash performs brace expansion by default
    // (`printf %s {a,b}` → `a b`, `{1..3}` → `1 2 3`). Re-enable it for
    // `--bash` so it matches /bin/bash.
    if matches!(shell_mode(), ShellMode::Bash) && !zsh_style_requested {
        zsh::ported::options::opt_state_set("ignorebraces", false);
        // bash always populates `$BASH_REMATCH` (array: [0]=whole match,
        // [1..]=capture groups) after `[[ str =~ re ]]`. zsh gates this on
        // the BASH_REMATCH option; turn it on for --bash.
        zsh::ported::options::opt_state_set("bashrematch", true);
        // Enable bash-only param expansion syntax (`${!var}` indirect,
        // `${v^^}` case-mod) in the subst layer.
        zsh::extensions::dash_mode::set_bash_mode(true);
    }

    if parity_mode_selected {
        // Use the process-local AtomicBool override instead of
        // exporting `ZSHRS_CACHE=0` in env. The env-var approach
        // imported `ZSHRS_CACHE` into paramtab during the c:893
        // env scan, leaking into `${(k)parameters}` and inflating
        // the count vs reference zsh by 1.
        zsh::extensions::script_cache::CACHE_DISABLED
            .store(true, std::sync::atomic::Ordering::Relaxed);
        tracing::info!(mode = ?shell_mode(), "parity mode: cache disabled via override, daemon disabled, plugin_cache replay disabled");
    }
    tracing::info!(mode = ?shell_mode(), "shell mode selected");

    // Handle --help (must be identical to zsh --help)
    if args.iter().any(|a| a == "--help") {
        print_help();
        return;
    }

    // Handle --version
    if args.iter().any(|a| a == "--version") {
        println!("zshrs {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // --ztest-worker: persistent test-worker subprocess for the
    // --ztest pool runner. Reads JSON requests from stdin, forks per
    // request, child runs the test in-process and writes one JSON
    // response per line to stdout. Port of stryke's --test-worker
    // mode (../strykelang/strykelang/cli_runners.rs:204). Must be
    // checked BEFORE any thread spawns in this process — raw
    // libc::fork inside the loop requires the main thread is alone.
    if args.iter().any(|a| a == "--ztest-worker") {
        std::process::exit(zsh::ztest::run_ztest_worker_loop());
    }

    // --ztest [paths...]: shell-level unit-test runner — worker-pool
    // architecture mirroring `stryke test` (../strykelang README
    // [0x0C-test]). Empty paths → discover `t/` then `tests/`. Each
    // test file is a shell script that calls `zassert_*` and ends
    // with `ztest_run`. Implementation: src/extensions/ztest.rs.
    //
    // Flags consumed:
    //   -j N    — N worker processes (default: num_cpus)
    //   -q      — quiet (suppress per-file banners)
    if let Some(idx) = args.iter().position(|a| a == "--ztest") {
        let rest = &args[idx + 1..];
        let mut j_threads: Option<String> = None;
        let mut quiet = false;
        let mut targets: Vec<String> = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            match rest[i].as_str() {
                "-j" => {
                    if i + 1 < rest.len() {
                        j_threads = Some(rest[i + 1].clone());
                        i += 2;
                        continue;
                    }
                }
                "-q" | "--quiet" => {
                    quiet = true;
                }
                t => targets.push(t.to_string()),
            }
            i += 1;
        }
        std::process::exit(zsh::ztest::run_ztests_pool(
            &targets,
            j_threads.as_deref(),
            quiet,
        ));
    }

    // (the `--daemon` arg is intercepted earlier in zshrs_main with a
    // pointer at the install paths; no second handler here.)

    // Handle --doctor (zshrs-exclusive, not available in --zsh or --posix)
    if args.iter().any(|a| a == "--doctor") {
        if is_zshrs_mode() {
            run_doctor();
        } else {
            eprintln!("zshrs: --doctor is only available in zshrs mode (not --zsh or --posix)");
            std::process::exit(1);
        }
        return;
    }

    // Handle --dump-zwc for debugging .zwc files
    if args.len() >= 3 && args[1] == "--dump-zwc" {
        if args.len() >= 4 {
            // Dump specific function
            if let Err(e) = zwc::dump_zwc_function(&args[2], &args[3]) {
                eprintln!("zshrs: {}: {}", args[2], e);
                std::process::exit(1);
            }
        } else {
            // List all functions
            if let Err(e) = zwc::dump_zwc_info(&args[2]) {
                eprintln!("zshrs: {}: {}", args[2], e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Handle --dump-tokens / --dump-ast / --dump-wordcode for parser-pipeline
    // debugging. Each takes one positional FILE arg (or `-` to read from
    // stdin) and prints the corresponding IR to stdout in the same canonical
    // format as the C-side `zshrs/zshrs_dump` module's `dumptokens` /
    // `dumpwordcode` builtins (so output can be diff'd byte-for-byte against
    // C zsh for parity verification).
    for &(flag, dumper) in &[
        (
            "--dump-tokens",
            zsh::dumpers::dump_tokens as fn(&str) -> String,
        ),
        ("--dump-ast", zsh::dumpers::dump_ast as fn(&str) -> String),
        (
            "--dump-wordcode",
            zsh::dumpers::dump_wordcode as fn(&str) -> String,
        ),
    ] {
        if args.len() >= 3 && args[1] == flag {
            let path = &args[2];
            let src = if path == "-" {
                let mut buf = String::new();
                if let Err(e) = io::stdin().read_to_string(&mut buf) {
                    eprintln!("zshrs: stdin: {}", e);
                    std::process::exit(1);
                }
                buf
            } else {
                match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("zshrs: {}: {}", path, e);
                        std::process::exit(1);
                    }
                }
            };
            print!("{}", dumper(&src));
            return;
        }
    }

    // --fmt: format zsh source (src/extensions/fmt.rs — the same
    // engine the LSP's textDocument/formatting uses, so CLI and IDE
    // Reformat agree byte-for-byte).
    //   zshrs --fmt [-w] [-t] [-i N] [FILE…]
    // No files → stdin to stdout. -w rewrites files in place.
    // -i N sets the indent width (default 4); -t indents with tabs.
    if let Some(fi) = args.iter().position(|a| a == "--fmt") {
        let mut write_in_place = false;
        let mut opts = zsh::fmt::FmtOptions::default();
        let mut files: Vec<String> = Vec::new();
        let mut j = fi + 1;
        while j < args.len() {
            match args[j].as_str() {
                "-w" => write_in_place = true,
                "-t" => opts.use_tabs = true,
                "-i" => {
                    if let Some(n) = args.get(j + 1).and_then(|s| s.parse().ok()) {
                        opts.indent_width = n;
                        j += 1;
                    } else {
                        eprintln!("zshrs: --fmt: -i requires a number");
                        std::process::exit(1);
                    }
                }
                f => files.push(f.to_string()),
            }
            j += 1;
        }
        let mut status = 0;
        if files.is_empty() {
            if write_in_place {
                eprintln!("zshrs: --fmt: -w requires file arguments");
                std::process::exit(1);
            }
            let mut src = String::new();
            use std::io::Read as _;
            if std::io::stdin().read_to_string(&mut src).is_err() {
                eprintln!("zshrs: --fmt: stdin is not valid UTF-8");
                std::process::exit(1);
            }
            print!("{}", zsh::fmt::format_source(&src, &opts));
        } else {
            for f in &files {
                match std::fs::read_to_string(f) {
                    Ok(src) => {
                        let formatted = zsh::fmt::format_source(&src, &opts);
                        if write_in_place {
                            if formatted != src {
                                if let Err(e) = std::fs::write(f, &formatted) {
                                    eprintln!("zshrs: --fmt: {}: {}", f, e);
                                    status = 1;
                                }
                            }
                        } else {
                            print!("{}", formatted);
                        }
                    }
                    Err(e) => {
                        eprintln!("zshrs: --fmt: {}: {}", f, e);
                        status = 1;
                    }
                }
            }
        }
        std::process::exit(status);
    }

    // --lsp: start the Language Server Protocol server on stdio.
    // Used by the IntelliJ plugin (editors/intellij) and any other LSP
    // client (Helix, Neovim, VS Code, etc.). Implementation lives in
    // src/extensions/lsp.rs.
    if args.iter().any(|a| a == "--lsp") {
        std::process::exit(zsh::lsp::run_lsp());
    }

    // --dap [HOST:PORT]: serve the Debug Adapter Protocol.
    //   --dap HOST:PORT  → connect back to the IDE's DAP listener over TCP
    //                      (the path JetBrains uses; stdout stays free).
    //   --dap            → serve DAP over stdio (for clients that spawn the
    //                      adapter as an executable, e.g. VS Code).
    // Implementation in src/extensions/dap.rs.
    if let Some(i) = args.iter().position(|a| a == "--dap") {
        // A following arg is the listener address only if it looks like one
        // (HOST:PORT); otherwise it's a script/flag and we serve over stdio.
        let addr = args
            .get(i + 1)
            .map(|s| s.as_str())
            .filter(|s| s.contains(':'));
        std::process::exit(zsh::dap::run_dap(addr));
    }

    // --tiers FILE: run the script, then report which fusevm execution tier
    // took each of its chunks — asked of fusevm's own eligibility and cache
    // predicates, so the answer comes from the compiler that would have done
    // the work. The script's own output precedes the report.
    if let Some(i) = args.iter().position(|a| a == "--tiers") {
        let Some(path) = args.get(i + 1) else {
            eprintln!("zshrs: --tiers: requires a script path");
            std::process::exit(1);
        };
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("zshrs: --tiers: {path}: {e}");
                std::process::exit(1);
            }
        };
        match zsh::tiers::report(&src) {
            Ok(r) => println!("{r}"),
            Err(e) => {
                eprintln!("zshrs: --tiers: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // --dump-reflection: emit the JSON consumed by the IntelliJ "zshrs"
    // reflection tool window. One top-level key per category.
    if args.iter().any(|a| a == "--dump-reflection") {
        println!("{}", zsh::lsp::dump_reflection_json());
        return;
    }

    // --dump-plugins: emit the JSON consumed by the IntelliJ External
    // Libraries view. Groups every entry in the plugin_cache SQLite
    // by inferred plugin manager (zinit / oh-my-zsh / prezto / antidote /
    // antigen / zplug / zsh-more-completions / zpwr / loose). Empty array
    // when the cache is empty (first run before any plugin is sourced).
    if args.iter().any(|a| a == "--dump-plugins") {
        println!("{}", zsh::plugin_cache::dump_plugins_json());
        return;
    }

    // --dump-reference-html: emit the HTML chapter sections that
    // docs/reference.html splices between its LSP-REFERENCE markers.
    // `scripts/update_reference_html.sh` calls this then rewrites the
    // doc in place.
    if args.iter().any(|a| a == "--dump-reference-html") {
        print!("{}", zsh::lsp::dump_reference_html());
        return;
    }

    // --docs NAME: render the same hover card the LSP would return for
    // NAME. Used by the IntelliJ tool window's docs popup, by shell
    // tab-completion (`zshrs --docs <TAB>` via completions/_zshrs),
    // and as a terminal-side cheatsheet (matches `stryke docs NAME`
    // shape — cyan header, dim separator, cyan inline-code, green
    // indented code blocks).
    if let Some(i) = args.iter().position(|a| a == "--docs") {
        if let Some(name) = args.get(i + 1) {
            let card = zsh::lsp::lookup_doc(name);
            if card.is_empty() {
                eprintln!("zshrs: no docs for {}", name);
                if let Some(s) = zsh::lsp::closest_name(name) {
                    eprintln!("zshrs: did you mean `{}`?", s);
                }
                std::process::exit(1);
            }
            // Colorize only when stdout is a real TTY — keep machine
            // pipelines (IntelliJ tool window's docs popup, scripts,
            // `zshrs --docs X | jq`) on the raw markdown. Toggle with
            // `--color always` / `--color never` if needed.
            use std::io::IsTerminal;
            let color_flag = args
                .iter()
                .position(|a| a == "--color")
                .and_then(|j| args.get(j + 1).map(String::as_str));
            let want_color = match color_flag {
                Some("always") => true,
                Some("never") => false,
                _ => std::io::stdout().is_terminal(),
            };
            print!("{}", render_doc_card(name, &card, want_color));
            return;
        }
    }

    // --gen-docs [PATH] [--out DIR]: walk a directory, find every
    // shell-source file, emit per-file Markdown reference docs under
    // --out (default `docs/`). Mirrors `stryke gen-docs` for the
    // zshrs CLI. The output is a `.md` per source file with the same
    // relative path under the output dir.
    if let Some(i) = args.iter().position(|a| a == "--gen-docs") {
        let rest: Vec<&str> = args.iter().skip(i + 1).map(String::as_str).collect();
        std::process::exit(run_gen_docs_subcommand(&rest));
    }

    // --names: emit every canonical name across builtins / keywords /
    // options / specials / compsys / extensions, one per line, sorted
    // and de-duplicated. Drives `compadd ${(f)"$(zshrs --names)"}` in
    // the `_zshrs` completer for `--docs <TAB>`. Fast path: iterates
    // the in-memory const tables, no I/O, ~1 ms cold.
    if args.iter().any(|a| a == "--names") {
        for n in zsh::lsp::all_canonical_names() {
            println!("{}", n);
        }
        return;
    }

    // ── c:Src/init.c:401-556 — `parseopts()` ───────────────────────────
    //
    // Walk the LEADING option words (`-…` / `+…`) the way C does and
    // resolve every option through the same two lookups C uses:
    //   * single letters      → `optlookupc` (Src/options.c:723) over the
    //                           `zshletters` table (Src/options.c:292)
    //   * `-o NAME`, `+o NAME`,
    //     `--NAME`, `+-NAME`  → `optlookup`  (Src/options.c:686)
    // Each resolution is recorded in argv order and replayed with
    // `dosetopt(optno, action, toplevel)` inside `apply_cli_flags`.
    // C applies inline at c:501 / c:526; the Rust bin has to defer
    // because the emulation mode (`--zsh`, `--bash`, …) is installed
    // into the option table by `enter_*_mode()` at that point and would
    // otherwise overwrite whatever the command line asked for. This is
    // the same ordering C gets from `parseopts_setemulate` (c:359)
    // running before the first option character is consumed.
    //
    // Before this port the block recognised only `-f`, `-x`, `-v` and a
    // separated `-o NAME`; every other option letter was dropped on the
    // floor. `zshrs -e script.sh` ran the script WITHOUT errexit, `-u`
    // never errored on an unset parameter, and `-n` executed the script
    // instead of only parsing it. A per-letter differential against
    // `zsh -f` showed 47 of the 57 letters in `zshletters` diverging.
    //
    // c:289-290 — the letter table C consults is `kshletters` when
    // SHOPTIONLETTERS is set, which is an OPT_BOURNE default (sh/ksh
    // emulation). C has the emulation installed before parseopts runs;
    // raise the flag up-front for the Bourne modes so `optlookupc`
    // picks the right table, and let `enter_*_mode()` re-establish the
    // complete option set afterwards.
    if matches!(
        shell_mode(),
        ShellMode::Ksh | ShellMode::Posix | ShellMode::Dash
    ) {
        zsh::ported::options::opt_state_set("shoptionletters", true);
    }

    // Resolved `(optno, action)` pairs in argv order. `action` is true
    // for a `-` word (set) and false for a `+` word (unset); a negative
    // `optno` carries the inverted-sense marker (`-n` ↔ EXECOPT), which
    // `dosetopt` flips at c:739-741.
    let mut option_actions: Vec<(i32, bool)> = Vec::new();
    // c:400 — `*cmdp`: set when a `-c` option letter was consumed, so
    // the dispatch below knows the first operand is the command string
    // and not a script path. Tracked here rather than re-scanned out of
    // the filtered argv because after `-b` (c:511-517, option break) a
    // literal `-c` among the operands is NOT the flag.
    let mut cmd_word_seen = false;
    let args: Vec<String> = {
        use zsh::ported::options::{optlookup, optlookupc};
        use zsh::ported::zsh_h::{isset, OPT_INVALID, SHOPTIONLETTERS};

        // c:411-417 — `WARN_OPTION` + `return 1`, which parseargs turns
        // into `exit(1)` at c:291. Message text matches C's wording
        // ("bad option: -%c" c:522, "no such option: %s" c:494) with
        // zshrs's program prefix.
        fn bad_option(msg: String) -> ! {
            eprintln!("zshrs: {}", msg);
            std::process::exit(1);
        }

        // zshrs-only long flags. These are NOT zsh options, so the
        // faithful c:511 `optlookup` would reject them; they are
        // consumed here after having been read off the raw argv by the
        // shell-mode scan (line 1415) and `--disasm` (line 1286).
        const ZSHRS_LONG_FLAGS: &[&str] = &[
            "zsh-compat",
            "zsh",
            "bash",
            "ksh",
            "mksh",
            "pdksh",
            "sh",
            "dash",
            "ash",
            "csh",
            "posix",
            "disasm",
        ];

        let mut out: Vec<String> = Vec::new();
        // c:277 — `argv0 = argzero = posixzero = *argv++;`
        if let Some(a0) = args.first() {
            out.push(a0.clone());
        }
        let mut i = 1usize;
        let mut optionbreak = false; // c:403
        'words: while i < args.len() {
            // c:418 — `while (!optionbreak && *argv &&
            //            (**argv == '-' || **argv == '+'))`
            if optionbreak {
                break;
            }
            let word = args[i].clone();
            if !(word.starts_with('-') || word.starts_with('+')) {
                break;
            }
            let action = word.starts_with('-'); // c:420
                                                // c:421-422 — `if (!argv[0][1]) *argv = "--";`: a bare `-`
                                                // or `+` is rewritten to `--`, which the character loop
                                                // below immediately reads as the end-of-options marker.
            let chars: Vec<char> = if word.chars().count() == 1 {
                vec!['-', '-']
            } else {
                word.chars().collect()
            };
            let mut p = 1usize; // c:423 `while (*++*argv)`
            while p < chars.len() {
                let ch = chars[p];
                if ch == '-' {
                    // c:425-429 — the pseudo-option `--` ends options.
                    if p + 1 == chars.len() {
                        i += 1;
                        break 'words;
                    }
                    // c:430-431 — `-` is only allowed immediately after
                    // the leading `-`/`+`; anywhere else is a bad
                    // option string.
                    if p != 1 {
                        bad_option(format!("bad option string: '{}'", word));
                    }
                    // c:432 — `++*argv` steps past the second dash.
                    let long: String = chars[p + 1..].iter().collect();
                    if ZSHRS_LONG_FLAGS.contains(&long.as_str()) {
                        break;
                    }
                    // c:447-455 `--version` / c:456-459 `--help` are
                    // served by the earlier return-on-match blocks in
                    // this bin, so they never reach here.
                    // c:460-471 `--emulate MODE` — the mode name is read
                    // by the shell-mode scan at line 1415; consume the
                    // flag and its argument.
                    if long == "emulate" {
                        i += 1; // c:462 `++argv`
                        if i >= args.len() {
                            bad_option("--emulate: argument required".to_string());
                        }
                        break; // c:470
                    }
                    // c:473-475 — `-` characters are allowed in long
                    // options; they map onto `_`.
                    let name = long.replace('-', "_");
                    // c:493-497 — `longoptions:` → optlookup + dosetopt.
                    let optno = optlookup(&name);
                    if optno == OPT_INVALID {
                        bad_option(format!("no such option: {}", long));
                    }
                    option_actions.push((optno, action));
                    break; // c:507
                }
                // c:511-517 — `-b` ends option processing at the end of
                // this word, but only while SHOPTIONLETTERS is unset (in
                // ksh/sh emulation `b` is an ordinary option letter).
                if ch == 'b' && !isset(SHOPTIONLETTERS) {
                    optionbreak = true; // c:516
                    p += 1;
                    continue;
                }
                // c:518-527 — `-c command`: the command string is the
                // NEXT argv word, consumed at c:549 (`doneoptions`).
                // Emit a normalised `-c` marker below so the dispatch
                // finds it regardless of clumping (`-fc`, `-ic`).
                if ch == 'c' {
                    cmd_word_seen = true; // c:524 `*cmdp = *argv`
                    p += 1;
                    continue;
                }
                // c:528-533 — `-o NAME` / `-oNAME`: the option name is
                // the rest of this word, or the next word when the rest
                // is empty.
                if ch == 'o' {
                    let attached: String = chars[p + 1..].iter().collect();
                    let name = if attached.is_empty() {
                        i += 1; // c:529 `argv++`
                        match args.get(i) {
                            Some(n) => n.clone(),
                            // c:531-532
                            None => bad_option("string expected after -o".to_string()),
                        }
                    } else {
                        attached
                    };
                    let optno = optlookup(&name); // c:493
                    if optno == OPT_INVALID {
                        bad_option(format!("no such option: {}", name)); // c:494
                    }
                    option_actions.push((optno, action)); // c:501
                    break; // c:507
                }
                // c:509-516 — whitespace inside an option word is only
                // legal if the whole remainder is whitespace.
                if ch.is_whitespace() {
                    if chars[p..].iter().any(|c| !c.is_whitespace()) {
                        bad_option(format!("bad option string: '{}'", word));
                    }
                    break; // c:515
                }
                // c:517-534 — a single option letter.
                let optno = optlookupc(ch); // c:520
                if optno == OPT_INVALID {
                    bad_option(format!("bad option: -{}", ch)); // c:521
                }
                option_actions.push((optno, action)); // c:526
                p += 1;
            }
            i += 1;
        }
        // c:548-553 — `doneoptions:` — when `-c` was seen the command
        // string is the first remaining word. Re-emit the `-c` marker
        // so the dispatch below reads `args[1] = "-c"`, `args[2] = cmd`.
        if cmd_word_seen {
            if i >= args.len() {
                // c:550-551 — `WARN_OPTION("string expected after -%s")`
                bad_option("string expected after -c".to_string());
            }
            out.push("-c".to_string());
        }
        // Remaining words are the script / command string and operands.
        while i < args.len() {
            out.push(args[i].clone());
            i += 1;
        }
        out
    };

    // Final requested state of a named option, derived from the ordered
    // action list (last write wins, exactly as C's sequential `dosetopt`
    // calls do). Used for the three flags that drive control flow in
    // this bin rather than only the option table.
    let final_opt_state = |name: &str| -> Option<bool> {
        let target = zsh::ported::options::optlookup(name);
        let mut state = None;
        for &(optno, action) in &option_actions {
            // c:739-741 — a negative optno inverts the requested value.
            let (idx, value) = if optno < 0 {
                (-optno, !action)
            } else {
                (optno, action)
            };
            if idx == target {
                state = Some(value);
            }
        }
        state
    };
    let enable_xtrace = final_opt_state("xtrace").unwrap_or(false);
    let enable_verbose = final_opt_state("verbose").unwrap_or(false);
    // `-f` / `+o rcs` / `--no-rcs` all resolve to RCS-off; that is the
    // flag that suppresses the startup files, so read it back off the
    // resolved actions rather than re-scanning argv text (which missed
    // the clumped `-fc` and `-o norcs` spellings).
    let no_rcs_flag = final_opt_state("rcs").map(|on| !on).unwrap_or(false);

    /// Apply CLI flags and shell mode to executor
    fn apply_cli_flags(
        executor: &mut ShellExecutor,
        xtrace: bool,
        verbose: bool,
        no_rcs: bool,
        opt_actions: &[(i32, bool)],
    ) {
        // Apply shell mode
        executor.zsh_compat = is_zsh_mode();
        executor.bash_compat = is_bash_mode();
        if is_dash_mode() {
            // dash is sh + strict subset; enter_dash_mode sets EMULATE_SH
            // AND raises DASH_STRICT (enter_posix_mode would clear it).
            executor.enter_dash_mode();
        } else if is_posix_mode() {
            executor.enter_posix_mode();
        }
        if is_ksh_mode() {
            executor.enter_ksh_mode();
        }
        if xtrace {
            zsh::ported::options::opt_state_set("xtrace", true);
        }
        if verbose {
            zsh::ported::options::opt_state_set("verbose", true);
        }
        if no_rcs {
            // Match zsh -f: RCS is turned off so `setopt` lists `norcs`.
            // zsh keeps globalrcs on (only the user-rcs files are
            // skipped). HASHDIRS is not touched here — it derives from
            // INTERACTIVE below, the way c:314-315 does it.
            zsh::ported::options::opt_state_set("rcs", false);
        }
        // c:Src/init.c:298 and c:526 — both the script-file dispatch
        // (`opts[INTERACTIVE] &= 1`) and `-c` (`new_opts[INTERACTIVE]
        // &= 1`) clear the default-on sentinel 2 → 0. Only an explicit
        // `-i` (which writes 1) survives that mask, so establish OFF
        // here and let the replayed option words below override it.
        // `apply_cli_flags` only runs on those two dispatch paths; the
        // no-script/no-command path goes through
        // `ported::init::parseargs`, which keeps the full 0/1/2 model.
        zsh::ported::options::opt_state_set("interactive", false);
        // c:Src/init.c:368 — `opts[USEZLE] = 1;` in
        // `parseopts_setemulate`, i.e. before the option words are read,
        // so an explicit `-Z` / `+Z` overrides it and `init_io` gets the
        // final say below.
        zsh::ported::options::opt_state_set("zle", true);
        // c:Src/init.c:501 / c:526 — `dosetopt(optno, action, toplevel,
        // new_opts)` for every option word, in argv order. `force` is
        // the C `toplevel` flag (1 here), so the startup-only options
        // INTERACTIVE / SHINSTDIN / SINGLECOMMAND / USEZLE are settable
        // from the command line exactly as they are in C's parseargs.
        for &(optno, action) in opt_actions {
            zsh::ported::options::dosetopt(optno, action as i32, 1);
        }
        // c:Src/init.c:312-315 — `if (opts[MONITOR] == 2)
        //   opts[MONITOR] = opts[INTERACTIVE]; if (opts[HASHDIRS]
        //   == 2) opts[HASHDIRS] = opts[INTERACTIVE];`. Both
        // MONITOR and HASHDIRS auto-derive from INTERACTIVE state.
        // Under `-c` mode, stdin is parsed from the cmd arg (not
        // tty) so INTERACTIVE=false, which means HASHDIRS=false.
        // Without this derivation, `setopt` no-args under `-c`
        // emitted nothing because hashdirs stayed at its emulate-
        // ZSH default ON, matching the no-divergence filter at
        // Src/options.c:462. Bug #87 in docs/BUGS.md.
        let interactive = zsh::ported::options::opt_state_get("interactive").unwrap_or(false);
        zsh::ported::options::opt_state_set("monitor", interactive);
        zsh::ported::options::opt_state_set("hashdirs", interactive);
        // c:Src/init.c:703-710 — `init_io`:
        //   if (interact) { init_shout();
        //                   if (!SHTTY || !shout) opts[USEZLE] = 0; }
        //   else opts[USEZLE] = 0;
        // The comment above it says "only use zle if SHTTY != -1", but
        // the code tests `!SHTTY`, i.e. SHTTY == 0 — and `movefd` never
        // hands back fd 0. `!shout` cannot fire either: `init_shout`
        // falls back to `shout = stderr` when SHTTY == -1 (c:735-740,
        // "Since we're interactive, it's nice to have somewhere to
        // write"). So the branch collapses to `USEZLE = interact`, which
        // is why `zsh -f -i -c …` reports `zle` on even with no
        // controlling terminal at all. Reproduce the observable rule,
        // not the comment.
        if !interactive {
            zsh::ported::options::opt_state_set("zle", false); // c:707/710
        }
        // c:Src/init.c:715-718 — `if (opts[MONITOR]) { if (SHTTY == -1)
        //   opts[MONITOR] = 0; … }`. MONITOR, unlike USEZLE, really does
        // need a terminal. SHTTY is whatever `init_io` would have
        // acquired: fd 0 or fd 1 when either is a tty (c:640, c:678),
        // else `/dev/tty` (c:682). The `-c` / script dispatch never
        // reaches `init_io` — only the no-command path does, through
        // `ported::init::zsh_main` — so make the same call here and
        // close the probe fd again, since this path never edits a line.
        let has_shtty = unsafe {
            libc::isatty(0) != 0 || libc::isatty(1) != 0 || {
                let fd = libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
                if fd >= 0 {
                    libc::close(fd);
                    true
                } else {
                    false
                }
            }
        };
        if !has_shtty {
            zsh::ported::options::opt_state_set("monitor", false); // c:718
        }
    }

    // c:Src/init.c:548-553 (`doneoptions:`) — `-c` takes the NEXT word
    // as the command string. It is not required to be the first option:
    // `zsh -i -c 'print hi'`, `zsh -l -c …` and the clumped `zsh -fic …`
    // all run the command. The option walk above already decided this
    // and re-emitted a normalised `-c` marker at index 1, so the index
    // is fixed — no second scan of the filtered argv, which used to
    // mistake a post-`-b` (option-break) `-c` operand for the flag.
    let cmd_idx = if cmd_word_seen { Some(1) } else { None };
    // Handle -c 'command' syntax
    if let Some(ci) = cmd_idx.filter(|ci| ci + 1 < args.len()) {
        let code = &args[ci + 1];

        let mut executor = ShellExecutor::new();
        apply_cli_flags(
            &mut executor,
            enable_xtrace,
            enable_verbose,
            no_rcs_flag,
            &option_actions,
        );
        // c:Src/init.c:1340 — `if (cmd)
        //                       setsparam("ZSH_EXECUTION_STRING",
        //                                 ztrdup_metafy(cmd));`
        // ZSH_EXECUTION_STRING carries the -c argument so user scripts
        // can introspect what they're running. p10k probes this for the
        // initial prompt-deferral decision; pre-c-mode-aware tooling
        // reads it to log invocations. Previously only setupvals() in
        // ported/init.rs set the env var, but setupvals isn't wired
        // into the bin entry path so the value was always empty.
        zsh::ported::params::setsparam("ZSH_EXECUTION_STRING", code);
        // c:Src/init.c:1535 — `execstring(cmd, 0, 1, "cmdarg")` pushes
        // "cmdarg" onto the zsh_eval_context stack BEFORE running the
        // -c command. ZSH_EVAL_CONTEXT is the `:`-joined view of that
        // stack tied to the `zsh_eval_context` array. For a non-nested
        // -c invocation the stack is just ["cmdarg"], so the readable
        // form is the literal string "cmdarg". Plugins (zinit's
        // `[[ $ZSH_EVAL_CONTEXT == *cmdarg* ]]` predicate, p10k's
        // turbo-mode hooks) gate on this.
        // ZSH_EVAL_CONTEXT carries PM_READONLY (declared at
        // params.rs special_params:836), so setsparam would be
        // rejected by assignstrvalue's PM_READONLY guard. Write
        // u_str directly — same pattern BUILTIN_SET_LINENO uses
        // for the LINENO bypass. C zsh's PM_SPECIAL GSU setfn
        // handles this implicitly; the Rust port lacks the GSU
        // vtable so internal writes bypass via direct paramtab
        // mutation.
        //
        // Push "cmdarg" onto the static `zsh_eval_context` stack AND
        // mirror into the tied `zsh_eval_context` array + the
        // `ZSH_EVAL_CONTEXT` scalar so `${zsh_eval_context[*]}`
        // expansion reads it. Bug #262 in docs/BUGS.md. Inlined here
        // (formerly a shared vm_helper fn; relocated into doshfunc +
        // this bin-entry callsite). No nested process below pops it —
        // the `-c` invocation owns the stack for the whole run.
        if let Ok(mut ctx) = zsh::ported::exec::zsh_eval_context.lock() {
            ctx.push("cmdarg".to_string());
            let joined = ctx.join(":");
            if let Ok(mut tab) = zsh::ported::params::paramtab().write() {
                if let Some(pm) = tab.get_mut("zsh_eval_context") {
                    pm.u_arr = Some(ctx.clone());
                    pm.node.flags &= !(zsh::ported::zsh_h::PM_UNSET as i32);
                }
                if let Some(pm) = tab.get_mut("ZSH_EVAL_CONTEXT") {
                    pm.u_str = Some(joined);
                    pm.node.flags &= !(zsh::ported::zsh_h::PM_UNSET as i32);
                }
            }
        }
        // POSIX `sh -c script [name [args...]]` semantics
        // (Src/init.c:271 + 479): the next non-option arg AFTER the
        // command string becomes $0; remaining args become $1, $2, …
        // When no name is supplied, $0 falls back to argv[0] (the
        // binary path) — matching `zsh -c '...'`'s behavior of
        // exposing the full path of the shell binary.
        let zero = if args.len() > ci + 2 {
            // `zshrs [opts] -c 'cmd' name args...` — layout relative to the
            // `-c` found above (`ci`), since the --zsh / -f / -x filter does
            // not necessarily leave `-c` at index 1:
            //   args[0]      = binary path
            //   args[ci]     = "-c"
            //   args[ci + 1] = the command string
            //   args[ci + 2] = $0 name
            //   args[ci + 3..] = $1, $2, …
            executor.set_pparams(args[ci + 3..].to_vec());
            args[ci + 2].clone()
        } else {
            // c:Src/init.c:271 — `posixzero = ztrdup(argv[0])`: `$0`
            // in `-c` mode is the kernel-supplied argv[0] of THIS
            // binary, in --zsh parity mode too. (A previous revision
            // probed the system zsh install path and reported THAT as
            // `$0` for byte-parity — faking the shell's identity.
            // Parity tests that compare `$0` text must normalize the
            // machine-specific binary path in the test row instead.)
            args[0].clone()
        };
        executor.set_scalar("0".to_string(), zero.clone());
        // Wire the GSU dispatch: `argzerogetfn` reads
        // `utils::argzero()` which returns this value via
        // `lookup_special_var("0")`. Without `set_argzero`, the GSU
        // dispatch returns None and reads fall back to
        // `executor.variables`, but the explicit-name case above
        // depends on the GSU side knowing the value too.
        zsh::ported::utils::set_argzero(Some(zero.clone()));

        // Per Src/init.c:479 — `-c` mode hardcodes
        //   `scriptname = scriptfilename = ztrdup("zsh")`
        // (literal short name, NOT argzero). In `--zsh` parity mode,
        // match C zsh exactly so PS4 / `%N` / xtrace prefixes byte-
        // match the reference. Plain zshrs `-c` keeps the binary
        // basename for branding. `$0` is argv[0] either way.
        let basename = if is_zsh_mode() {
            "zsh".to_string()
        } else {
            std::path::Path::new(&zero)
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.trim_start_matches('-').to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "zshrs".to_string())
        };
        executor.scriptname = Some(basename.clone());
        // scriptfilename mirrors C init.c:479 `scriptname =
        // scriptfilename = ztrdup("zsh")` — both seeded to the
        // basename in -c mode. scriptfilename stays put when we
        // enter a function (only scriptname mutates), so PS4's
        // %x continues to show the file basename inside function
        // bodies.
        executor.scriptfilename = Some(basename.clone());
        // Propagate to the canonical `SCRIPTNAME` static read by
        // prompt expansion (`%N`/`%x`) via `utils::scriptname_get`.
        // Without this, `printprompt4` saw None and `%N`/`%x` fell
        // back to argzero — the full binary path — making xtrace
        // emit `/Users/.../zshrs` instead of `zsh`.
        zsh::ported::utils::set_scriptname(Some(basename));

        // Source zshenv per Src/init.c:1473 (GLOBAL_ZSHENV) +
        // Src/init.c:1489 (`sourcehome(".zshenv")`). C zsh sources
        // both /etc/zshenv (always) and ~/.zshenv (when RCS is set
        // and not PRIVILEGED) BEFORE running the `-c` cmd through
        // `execstring(cmd, …)` at init.c:1535. Skipping zshenv broke
        // any user setup that exports env (PS4, PATH, locale, …)
        // from .zshenv — a `zshrs -fx -c '…'` invocation showed the
        // C-default `+%N:%i> ` prefix instead of the user's PS4.
        //
        // login + interactive RC files (zprofile / zshrc / zlogin)
        // are NOT sourced in `-c` mode, matching the C source's
        // `if (islogin)` / `if (interact)` gates at init.c:1491,
        // 1499, 1507 — `-c` is non-interactive non-login.
        source_startup_files(&mut executor, false, false, no_rcs_flag);

        // Skip-configs apply: when the daemon is up + has zshrs
        // canonical state, apply it here so `zshrs -c 'gst'`
        // resolves the alias the same way an interactive shell
        // would. This runs AFTER zshenv so user env wins on
        // collision (canonical state seeds defaults; `.zshenv` is
        // the user's authoritative source).
        #[cfg(feature = "daemon")]
        if zsh::daemon_presence::should_skip_configs() {
            let _applied = zsh::canonical_apply::apply_all(&mut executor);
        }

        maybe_source_zshrs_startup_config(&mut executor, no_rcs_flag);

        // Long-cmd-started watchdog (-c path mirrors the interactive loop).
        #[cfg(feature = "daemon")]
        let completed_c = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        #[cfg(feature = "daemon")]
        {
            let line_owned = code.clone();
            let cwd_owned = std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string());
            let completed_c = std::sync::Arc::clone(&completed_c);
            let threshold_secs: u64 = std::env::var("ZSHRS_LONG_CMD_PRE_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(threshold_secs));
                if !completed_c.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = zsh::daemon::client::call_once_no_spawn(
                        "cmd_started",
                        serde_json::json!({
                            "line": line_owned,
                            "cwd": cwd_owned,
                            "shell_id": 0u64,
                        }),
                    );
                }
            });
        }

        let start = Instant::now();
        let result = executor.execute_script(code);
        #[cfg(feature = "daemon")]
        completed_c.store(true, std::sync::atomic::Ordering::SeqCst);
        let duration_ns_total = start.elapsed().as_nanos() as i64;
        let duration = duration_ns_total / 1_000_000;

        // Track in local history
        if let Some(ref engine) = executor.history {
            let cwd = std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string());
            if let Ok(id) = engine.add(code, cwd.as_deref()) {
                let _ = engine.update_last(id, duration, executor.last_status());
            }
        }

        // Daemon history_append (broadcasts long_cmd_complete on its end).
        #[cfg(feature = "daemon")]
        {
            let cwd = std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string());
            let _ = zsh::daemon::client::call_once_no_spawn(
                "history_append",
                serde_json::json!({
                    "line": code,
                    "exit_code": executor.last_status() as i64,
                    "cwd": cwd,
                    "duration_ns": duration_ns_total,
                }),
            );
        }

        if let Err(e) = result {
            // c:Src/init.c — `__SILENCED__` sentinel from
            // `execute_script_zsh_pipeline` means the parser already
            // emitted the diagnostic via zerr; binary-side print
            // would double it. Bug #142 in docs/BUGS.md.
            if e != "__SILENCED__" {
                eprintln!("zshrs: {}", e);
            }
            std::process::exit(1);
        }
        std::process::exit(executor.last_status());
        #[allow(unreachable_code)]
        return;
    }

    // Handle script file argument.
    // c:Src/init.c:296-303 — after `doneoptions:` the first remaining
    // word is the script, with NO leading-dash test: `--` and `-b` end
    // option processing, so `zsh -- -c` runs a FILE named `-c` (and
    // reports "can't open input file: -c"). The old `!starts_with('-')`
    // guard silently fell through to the interactive shell instead.
    // Every genuine option word has already been consumed — and an
    // unrecognised one exits at c:521 — so anything left here is an
    // operand whatever it looks like.
    if args.len() >= 2 {
        let mut executor = ShellExecutor::new();
        apply_cli_flags(
            &mut executor,
            enable_xtrace,
            enable_verbose,
            no_rcs_flag,
            &option_actions,
        );
        // Port from Src/init.c:295-306 + Src/init.c:1368-1370.
        // In script mode the parsed argv is split as:
        //   argv[0] = shell binary       (from init.c:271)
        //   argv[1] = runscript          (from init.c:301: `*runscript = *argv`)
        //   argv[2..] = paramlist        (from init.c:305-306, becomes pparams)
        // Before running the script, init.c:1369 does
        //   `argzero = ztrdup(runscript)` — i.e. `$0` becomes the
        // verbatim script path the user passed (NOT canonicalized).
        executor.set_scalar("0".to_string(), args[1].clone());
        executor.set_pparams(args.iter().skip(2).cloned().collect());
        // c:Src/init.c:1330 — `if (runscript) setsparam("ZSH_SCRIPT",
        // ztrdup(runscript));`. Also bug #25 in docs/BUGS.md.
        // setupshin in ported/init.rs runs at a different init path
        // (it's currently dead code via init_exec which the bin entry
        // doesn't call); set ZSH_SCRIPT here at the bin-level
        // script-dispatch site so user scripts can introspect their
        // own path. Mirrors the -c branch above which sets
        // ZSH_EXECUTION_STRING.
        zsh::ported::params::setsparam("ZSH_SCRIPT", &args[1]);
        // c:Src/init.c:1572 — `scriptname = ztrdup(runscript);` updates
        //   the C global `scriptname` to the script path; PS4's `%N`
        //   reads this (Src/prompt.c:555 promptpath(scriptname, ...))
        //   so xtrace prefixes show the script name at top level and
        //   switch to function names inside fns (via exec.c:5903 stash/
        //   restore). Bug #318 in docs/BUGS.md — script mode left
        //   SCRIPTNAME=None so `%N` fell back to ZSH_NAME ("zsh").
        zsh::ported::utils::set_scriptname(Some(args[1].clone()));
        // c:Src/init.c:1573 — `scriptfilename = ztrdup(runscript);`
        // — PS4's `%x` reads this (Src/prompt.c:931 path). Without
        // the matching write, `zshrs -x script.zsh` left the
        // scriptfilename TLS at "zsh" (the executor seed at
        // vm_helper.rs:1318), so xtrace col 1 showed "zsh" instead
        // of the script path for top-level lines.
        zsh::ported::utils::set_scriptfilename(Some(args[1].clone()));
        // c:Src/init.c:965 — `setsparam("ZSH_ARGZERO", ztrdup(posixzero));`
        // posixzero is the script path under script mode (or the
        // shell binary path under -c/interactive). In zshrs's script
        // dispatch posixzero is the script path (args[1]); the bin
        // entry's setupvals in ported/init.rs only runs through
        // init_exec which isn't reached on this code path, so set
        // ZSH_ARGZERO directly. Without this, ZSH_ARGZERO carried
        // argv[0] (the zshrs binary path) instead of the script path.
        // c:Src/init.c:220 — `execode(prog, 0, 0, toplevel ? "toplevel" : "file");`
        // The main input loop names its context "toplevel", so a SCRIPT FILE
        // (and stdin) runs with `ZSH_EVAL_CONTEXT=toplevel` where `-c` uses
        // "cmdarg" (c:init.c:1535). zshrs pushed "cmdarg" only on the `-c`
        // path and pushed NOTHING here, so a script file saw an EMPTY context
        // and every nested construct lost its base too — `shfunc` instead of
        // `toplevel:shfunc`, `cmdsubst` instead of `toplevel:cmdsubst`.
        // No pop: like the `-c` push, the script invocation owns the stack for
        // the whole run. Bug #1067.
        if let Ok(mut ctx) = zsh::ported::exec::zsh_eval_context.lock() {
            if ctx.is_empty() {
                ctx.push("toplevel".to_string());
                let joined = ctx.join(":");
                if let Ok(mut tab) = zsh::ported::params::paramtab().write() {
                    if let Some(pm) = tab.get_mut("zsh_eval_context") {
                        pm.u_arr = Some(ctx.clone());
                        pm.node.flags &= !(zsh::ported::zsh_h::PM_UNSET as i32);
                    }
                    if let Some(pm) = tab.get_mut("ZSH_EVAL_CONTEXT") {
                        pm.u_str = Some(joined);
                        pm.node.flags &= !(zsh::ported::zsh_h::PM_UNSET as i32);
                    }
                }
            }
        }
        zsh::ported::params::setsparam("ZSH_ARGZERO", &args[1]);
        match executor.execute_script_file(&args[1]) {
            Err(e) => {
                if e != "__SILENCED__" {
                    eprintln!("zshrs: {}: {}", args[1], e);
                }
                std::process::exit(1);
            }
            // c:Src/init.c:234 — loop() breaks on errflag in a
            // non-interactive shell and zsh_main exits with the
            // UNTOUCHED lastval; a clean run also exits with the
            // last command's status. The previous bare `return;`
            // here exited 0 even when the script aborted on a
            // readonly reassign (zsh 5.9 exits 1) — same driver bug
            // for any script whose final command fails.
            Ok(status) => std::process::exit(status),
        }
    }

    tracing::info!(
        startup_ms = startup_t0.elapsed().as_millis() as u64,
        "startup complete, entering main loop"
    );

    // Recording-staleness oracle: the startup path ignores rc files and
    // replays the recorder shard, so an edited `.zshrc` is invisible until
    // `zshrs record` re-runs. Log it (never printed to the tty — see the
    // no-startup-chatter rule) so the "I edited config and forgot" case is
    // discoverable via `~/.cache/zshrs/zshrs.log` + `--doctor` instead of
    // silently serving yesterday's environment. One directory listing + a
    // few stats; no IPC, no re-source.
    if let Some(stale_rc) = zsh::daemon_presence::recording_staleness() {
        tracing::warn!(
            rc = %stale_rc,
            "recording is stale: {} is newer than the recorded environment — run `zshrs record` to refresh",
            stale_rc
        );
    }

    // Check if stdin is a TTY
    // Faithful entry: zsh's `main()` (Src/main.c:114) is just
    // `return zsh_main(argc, argv)` — there is NO tty/non-tty branch and
    // NO separate non-interactive driver. `zsh_main` (Src/init.c:1855)
    // decides interactivity internally via `parseargs` (isatty → the
    // `interactive`/`SHINSTDIN` options), and `loop()` reads SHIN
    // identically whether stdin is a terminal, a pipe, or a redirected
    // file. So a single call covers both `zshrs` at a terminal and
    // `cmd | zshrs` / `zshrs < file`.
    //
    // Create the long-lived session executor and register it so loop()'s
    // `execode` (init.c:220) runs each parsed program through the fusevm
    // VM. The executor must outlive zsh_main; it never drops because
    // zsh_main exits the process from inside loop().
    let executor = Box::leak(Box::new(ShellExecutor::new()));
    zsh::ported::exec::install_session_executor(executor);
    let argv: Vec<String> = std::env::args().collect();
    std::process::exit(zsh::ported::init::zsh_main(argv.len() as i32, &argv));
}

/// zshrs --doctor: full diagnostic report of shell health, caches, and performance.
fn run_doctor() {
    let green = |s: &str| format!("\x1b[32m{}\x1b[0m", s);
    let red = |s: &str| format!("\x1b[31m{}\x1b[0m", s);
    let yellow = |s: &str| format!("\x1b[33m{}\x1b[0m", s);
    let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
    let dim = |s: &str| format!("\x1b[2m{}\x1b[0m", s);

    println!("{}", bold("zshrs doctor"));
    println!("{}", dim(&"=".repeat(60)));
    println!();

    // --- Version & Environment ---
    println!("{}", bold("Environment"));
    println!("  version:    zshrs {}", env!("CARGO_PKG_VERSION"));
    println!("  pid:        {}", std::process::id());
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "?".to_string());
    println!("  cwd:        {}", cwd);
    println!(
        "  shell:      {}",
        std::env::var("SHELL").unwrap_or_else(|_| "?".to_string())
    );
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    println!("  cpus:       {}", cpus);
    let pool_size = cpus.clamp(2, 18);
    println!("  pool size:  {}", pool_size);
    println!();

    // --- PATH ---
    println!("{}", bold("PATH"));
    let path_var = std::env::var("PATH").unwrap_or_default();
    let path_dirs: Vec<&str> = path_var.split(':').filter(|s| !s.is_empty()).collect();
    let mut path_ok = 0usize;
    let mut path_missing = 0usize;
    let mut path_cmds = 0usize;
    for dir in &path_dirs {
        if std::path::Path::new(dir).is_dir() {
            path_ok += 1;
            if let Ok(entries) = std::fs::read_dir(dir) {
                path_cmds += entries.count();
            }
        } else {
            path_missing += 1;
        }
    }
    println!(
        "  directories: {} total, {} {}, {} {}",
        path_dirs.len(),
        path_ok,
        green("valid"),
        path_missing,
        if path_missing > 0 {
            red("missing")
        } else {
            green("missing")
        },
    );
    println!("  commands:    ~{}", path_cmds);
    if path_missing > 0 {
        for dir in &path_dirs {
            if !std::path::Path::new(dir).is_dir() {
                println!("  {} PATH entry does not exist: {}", red("!"), dir);
            }
        }
    }
    println!();

    // --- FPATH ---
    println!("{}", bold("FPATH"));
    let fpath_var = std::env::var("FPATH").unwrap_or_default();
    let fpath_dirs: Vec<&str> = fpath_var.split(':').filter(|s| !s.is_empty()).collect();
    let mut fpath_ok = 0usize;
    let mut fpath_missing = 0usize;
    let mut fpath_files = 0usize;
    for dir in &fpath_dirs {
        if std::path::Path::new(dir).is_dir() {
            fpath_ok += 1;
            if let Ok(entries) = std::fs::read_dir(dir) {
                fpath_files += entries.count();
            }
        } else {
            fpath_missing += 1;
        }
    }
    println!(
        "  directories:   {} total, {} {}, {} {}",
        fpath_dirs.len(),
        fpath_ok,
        green("valid"),
        fpath_missing,
        if fpath_missing > 0 {
            red("missing")
        } else {
            green("missing")
        },
    );
    println!("  function files: {}", fpath_files);
    println!();

    // --- Caches (rkyv-mmapped) ---
    // Per docs/DESIGN_GOALS.md:13 and docs/DAEMON.md:226, the only
    // shell cache layer is rkyv-mmapped bytecode under
    // `~/.zshrs/images/` with the top-level `~/.zshrs/index.rkyv`
    // (fq_name → shard_id, generation, byte_offset). Hot lookups
    // never hit SQLite — clients mmap rkyv exclusively.
    println!("{}", bold("Caches (rkyv-mmapped)"));
    let zshrs_dir = dirs::home_dir()
        .map(|h| h.join(".zshrs"))
        .unwrap_or_else(|| PathBuf::from("/tmp/.zshrs"));
    let index_rkyv = zshrs_dir.join("index.rkyv");
    if index_rkyv.exists() {
        let size = std::fs::metadata(&index_rkyv).map(|m| m.len()).unwrap_or(0);
        println!(
            "  index:       {} {}  {}",
            index_rkyv.display(),
            format_bytes(size),
            green("OK")
        );
    } else {
        println!(
            "  index:       {} {}",
            index_rkyv.display(),
            yellow("(absent — daemon has not built shards yet)")
        );
    }
    let images_dir = zshrs_dir.join("images");
    if images_dir.is_dir() {
        let mut shards: Vec<(String, u64)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&images_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("rkyv") {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let name = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    shards.push((name, size));
                }
            }
        }
        shards.sort();
        let total: u64 = shards.iter().map(|(_, s)| *s).sum();
        println!(
            "  images/:     {} shards, {} total",
            shards.len(),
            format_bytes(total)
        );
        for (name, size) in &shards {
            println!("               {} {}", format_bytes(*size), name);
        }
    } else {
        println!(
            "  images/:     {} {}",
            images_dir.display(),
            yellow("(absent)")
        );
    }

    // Legacy single-file shards still in active shell-side use until
    // daemon hydration migrates them under images/.
    if let Some((count, bytes)) = zsh::script_cache::stats() {
        let path = zsh::script_cache::default_cache_path();
        println!(
            "  scripts:     {} entries, {}  {}",
            count,
            format_bytes(bytes as u64),
            dim(&format!("{}", path.display()))
        );
    }
    let autoload_count = zsh::autoload_cache::entry_count();
    if autoload_count > 0 {
        let path = zsh::autoload_cache::default_cache_path();
        println!(
            "  autoloads:   {} functions  {}",
            autoload_count,
            dim(&format!("{}", path.display()))
        );
    }
    println!();

    // --- SQLite (read-only mirrors) ---
    // Same directory, different job: daemon-maintained copies you can
    // query with SQL or `dbview`. They are NOT the bytecode cache and
    // are NOT read when deciding cache hit/miss or when running
    // compiled code. The numbers below are inspection-only.
    println!("{}", bold("SQLite (read-only mirrors)"));
    println!(
        "  {}",
        dim("daemon-maintained; not read on cache lookup / hot path")
    );

    let compsys_path = zsh::compsys::cache::default_cache_path();
    if compsys_path.exists() {
        let size = std::fs::metadata(&compsys_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let count = CompsysCache::open(&compsys_path)
            .ok()
            .map(|c| zsh::compsys::cache_entry_count(&c))
            .unwrap_or(0);
        println!(
            "  compsys.db:  {} completions, {}  {}",
            count,
            format_bytes(size),
            dim("mirror"),
        );
    } else {
        println!(
            "  compsys.db:  {}",
            yellow("not found — run compinit to create the mirror")
        );
    }

    let plugin_path = zsh::plugin_cache::default_cache_path();
    if plugin_path.exists() {
        let size = std::fs::metadata(&plugin_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let (plugins, functions) = zsh::plugin_cache::PluginCache::open(&plugin_path)
            .map(|c| c.stats())
            .unwrap_or((0, 0));
        println!(
            "  plugins.db:  {} plugins, {} functions, {}  {}",
            plugins,
            functions,
            format_bytes(size),
            dim("mirror"),
        );

        // Stale plugin diagnostic — file mtime no longer matches the
        // mirror's stored mtime. Indicates the rkyv shard may be out
        // of date and needs daemon rehydration.
        if let Ok(cache) = zsh::plugin_cache::PluginCache::open(&plugin_path) {
            let stale = count_stale_plugins(&cache);
            if stale > 0 {
                println!(
                    "               {} {} plugin(s) stale in mirror — rkyv shard may need rehydration",
                    yellow("!"),
                    stale
                );
            }
        }
    } else {
        println!(
            "  plugins.db:  {}",
            yellow("not found — source a file to create the mirror")
        );
    }
    println!();

    // --- History ---
    // History is a durable command record, not a cache. Reported in
    // its own section to make that distinction visible.
    println!("{}", bold("History"));
    let hist_path = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zshrs/history.db");
    if hist_path.exists() {
        let size = std::fs::metadata(&hist_path).map(|m| m.len()).unwrap_or(0);
        let count = zsh::history::HistoryEngine::new()
            .ok()
            .and_then(|e| e.count().ok())
            .unwrap_or(0);
        println!(
            "  history.db:  {} entries, {}  {}",
            count,
            format_bytes(size),
            green("OK"),
        );
    } else {
        println!("  history.db:  {}", yellow("not found"));
    }
    println!();

    // --- Log file ---
    println!("{}", bold("Log"));
    let log_path = zsh::log::log_path();
    if log_path.exists() {
        let size = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
        let lines = std::fs::read_to_string(&log_path)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        println!(
            "  {}  {} lines, {}",
            log_path.display(),
            lines,
            format_bytes(size)
        );
    } else {
        println!("  {}", dim("no log file yet"));
    }
    println!();

    // --- Startup files ---
    println!("{}", bold("Startup Files"));
    let zdotdir = std::env::var("ZDOTDIR")
        .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()));
    let startup_files = [
        (zsh::global_rc::global_rc_path("/etc/zshenv"), true),
        (format!("{}/.zshenv", zdotdir), false),
        (zsh::global_rc::global_rc_path("/etc/zprofile"), false),
        (format!("{}/.zprofile", zdotdir), false),
        (zsh::global_rc::global_rc_path("/etc/zshrc"), false),
        (format!("{}/.zshrc", zdotdir), false),
        (zsh::global_rc::global_rc_path("/etc/zlogin"), false),
        (format!("{}/.zlogin", zdotdir), false),
    ];
    for (path, _always) in &startup_files {
        let p = std::path::Path::new(path);
        if p.exists() {
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            let cached = is_script_cached(&plugin_path, path);
            let cache_status = if cached {
                green("cached")
            } else {
                yellow("uncached")
            };
            println!(
                "  {} {} {}  [{}]",
                green("*"),
                path,
                dim(&format!("({})", format_bytes(size))),
                cache_status,
            );
        } else {
            // Both `always`-flagged and not-found render the same dim
            // line; the distinction is meaningful only for the
            // present/up-to-date branch above.
            println!("  {} {}", dim("-"), dim(path));
        }
    }
    // Recording staleness — an rc file edited since the last recording.
    if !zsh::daemon_presence::recording_present() {
        println!(
            "  {} recording: {}",
            dim("-"),
            dim("none (rc files sourced normally)"),
        );
    } else {
        match zsh::daemon_presence::recording_staleness() {
            Some(rc) => println!(
                "  {} recording: {} ({} is newer — run `zshrs record`)",
                yellow("!"),
                yellow("STALE"),
                rc,
            ),
            None => println!("  {} recording: {}", green("*"), green("up to date")),
        }
    }
    println!();

    // --- Profiling ---
    println!("{}", bold("Profiling Features"));
    println!(
        "  chrome tracing: {}",
        if zsh::log::profiling_enabled() {
            green("enabled")
        } else {
            dim("disabled (build with --features profiling)")
        }
    );
    println!(
        "  flamegraph:     {}",
        if zsh::log::flamegraph_enabled() {
            green("enabled")
        } else {
            dim("disabled (build with --features flamegraph)")
        }
    );
    println!(
        "  prometheus:     {}",
        if zsh::log::prometheus_enabled() {
            green("enabled")
        } else {
            dim("disabled (build with --features prometheus)")
        }
    );
    println!(
        "  ZSHRS_LOG:      {}",
        std::env::var("ZSHRS_LOG").unwrap_or_else(|_| "info (default)".to_string())
    );
    println!();

    // --- Startup benchmark ---
    println!("{}", bold("Startup Benchmark"));
    let t0 = Instant::now();
    let mut executor = ShellExecutor::new();
    let init_ms = t0.elapsed().as_millis();
    println!("  executor init:  {}ms", init_ms);

    let t1 = Instant::now();
    executor.drain_compinit_bg();
    let drain_ms = t1.elapsed().as_millis();
    println!("  compinit drain: {}ms", drain_ms);

    let total = init_ms + drain_ms;
    let status = if total < 30 {
        green(&format!("{}ms — excellent", total))
    } else if total < 100 {
        yellow(&format!("{}ms — good", total))
    } else {
        red(&format!("{}ms — slow", total))
    };
    println!("  total:          {}", status);
    println!();

    // --- Summary ---
    println!("{}", bold("Summary"));
    let mut issues = 0;
    if path_missing > 0 {
        println!("  {} {} PATH entries missing", red("!"), path_missing);
        issues += 1;
    }
    if fpath_missing > 0 {
        println!("  {} {} FPATH entries missing", red("!"), fpath_missing);
        issues += 1;
    }
    if !hist_path.exists() {
        println!("  {} no history database", yellow("!"));
        issues += 1;
    }
    if !compsys_path.exists() {
        println!("  {} no completion cache", yellow("!"));
        issues += 1;
    }
    if total > 100 {
        println!("  {} startup > 100ms", red("!"));
        issues += 1;
    }
    if issues == 0 {
        println!("  {} all checks passed", green("*"));
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn count_stale_plugins(cache: &zsh::plugin_cache::PluginCache) -> usize {
    cache.count_stale()
}

fn is_script_cached(plugin_db_path: &std::path::Path, script_path: &str) -> bool {
    if !plugin_db_path.exists() {
        return false;
    }
    let cache = match zsh::plugin_cache::PluginCache::open(plugin_db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Some((mt_s, mt_ns)) = zsh::plugin_cache::file_mtime(std::path::Path::new(script_path)) {
        cache.check(script_path, mt_s, mt_ns).is_some()
    } else {
        false
    }
}

fn get_zdotdir() -> PathBuf {
    std::env::var("ZDOTDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
}

/// The system-wide startup file for `name`, resolved through
/// [`zsh::global_rc::global_rc_path`] so the Debian/Ubuntu
/// `/etc/zsh/…` layout is picked up on those platforms.
fn global_rc(name: &str) -> PathBuf {
    PathBuf::from(zsh::global_rc::global_rc_path(&format!("/etc/{name}")))
}

/// Source zsh startup files in correct order per zshall(1) STARTUP/SHUTDOWN FILES
///
/// Behavior is controlled by RCS and GLOBAL_RCS options:
/// - RCS (default: on) - if unset, no startup files are read
/// - GLOBAL_RCS (default: on) - if unset, the system-wide files are skipped
///
/// The system-wide paths below are written `/etc/…` for brevity; they are
/// resolved through [`global_rc`], which picks the Debian `/etc/zsh/…`
/// layout when that is what the platform uses.
///
/// Order for login shell:
///   1. /etc/zshenv (always, cannot be overridden - even with -f)
///   2. $ZDOTDIR/.zshenv
///   3. /etc/zprofile (login only)
///   4. $ZDOTDIR/.zprofile (login only)
///   5. /etc/zshrc (interactive only)
///   6. $ZDOTDIR/.zshrc (interactive only)
///   7. /etc/zlogin (login only)
///   8. $ZDOTDIR/.zlogin (login only)
///
/// If file.zwc exists and is newer than file, the compiled version is used.
///
/// Optimization: all startup file contents are read into memory in parallel
/// (overlapping disk I/O), then executed sequentially in the correct order.
fn source_startup_files(
    executor: &mut ShellExecutor,
    is_login: bool,
    is_interactive: bool,
    no_rcs: bool,
) {
    let zdotdir = get_zdotdir();

    // Build the ordered list of candidate startup files.
    // We read ALL of them in parallel to overlap disk latency, but execute
    // sequentially and honor RCS/GLOBAL_RCS checks between phases.
    let mut candidates: Vec<PathBuf> = Vec::with_capacity(8);

    // Phase 0: /etc/zshenv — always read
    candidates.push(global_rc("zshenv"));

    if !no_rcs {
        // Phase 1: user .zshenv
        candidates.push(zdotdir.join(".zshenv"));

        // Phase 2: login profile files
        if is_login {
            candidates.push(global_rc("zprofile"));
            candidates.push(zdotdir.join(".zprofile"));
        }

        // Phase 3: interactive rc files
        if is_interactive {
            candidates.push(global_rc("zshrc"));
            candidates.push(zdotdir.join(".zshrc"));
        }

        // Phase 4: login files (after zshrc)
        if is_login {
            candidates.push(global_rc("zlogin"));
            candidates.push(zdotdir.join(".zlogin"));
        }
    }

    // --- Parallel read phase: read all files at once on background threads ---
    let read_start = std::time::Instant::now();
    let file_count = candidates.len();

    let handles: Vec<std::thread::JoinHandle<(PathBuf, Option<String>)>> = candidates
        .into_iter()
        .map(|path| {
            std::thread::spawn(move || {
                let contents = if path.exists() {
                    std::fs::read_to_string(&path).ok()
                } else {
                    None
                };
                (path, contents)
            })
        })
        .collect();

    // Collect results in order (handles are in insertion order)
    let preloaded: Vec<(PathBuf, Option<String>)> = handles
        .into_iter()
        .map(|h| h.join().unwrap_or_else(|_| (PathBuf::new(), None)))
        .collect();

    tracing::debug!(
        files = file_count,
        read_ms = read_start.elapsed().as_millis() as u64,
        "startup files parallel read complete"
    );

    // --- Sequential execution phase: execute in correct order with RCS checks ---

    // Phase 0: /etc/zshenv — always
    if let Some((path, Some(text))) = preloaded.first() {
        source_from_memory(executor, path, text);
    }

    if no_rcs {
        return;
    }

    // Check RCS after /etc/zshenv
    if !zsh::ported::options::opt_state_get("rcs").unwrap_or(true) {
        return;
    }

    // Phase 1: $ZDOTDIR/.zshenv
    let mut idx = 1;
    if idx < preloaded.len() {
        if let Some(ref text) = preloaded[idx].1 {
            source_from_memory(executor, &preloaded[idx].0, text);
        }
        idx += 1;
    }

    // Re-check RCS after .zshenv
    if !zsh::ported::options::opt_state_get("rcs").unwrap_or(true) {
        return;
    }

    // Phase 2: login profile files
    if is_login {
        // /etc/zprofile
        if idx < preloaded.len() {
            if zsh::ported::options::opt_state_get("globalrcs").unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            idx += 1;
        }
        // $ZDOTDIR/.zprofile
        if idx < preloaded.len() {
            if zsh::ported::options::opt_state_get("rcs").unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            idx += 1;
        }
    }

    // Re-check RCS
    if !zsh::ported::options::opt_state_get("rcs").unwrap_or(true) {
        return;
    }

    // Phase 3: interactive rc files
    if is_interactive {
        // /etc/zshrc
        if idx < preloaded.len() {
            if zsh::ported::options::opt_state_get("globalrcs").unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            idx += 1;
        }
        // $ZDOTDIR/.zshrc
        if idx < preloaded.len() {
            if zsh::ported::options::opt_state_get("rcs").unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            idx += 1;
        }
    }

    // Re-check RCS
    if !zsh::ported::options::opt_state_get("rcs").unwrap_or(true) {
        return;
    }

    // Phase 4: login files (after zshrc)
    if is_login {
        // /etc/zlogin
        if idx < preloaded.len() {
            if zsh::ported::options::opt_state_get("globalrcs").unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            idx += 1;
        }
        // $ZDOTDIR/.zlogin
        if idx < preloaded.len() {
            if zsh::ported::options::opt_state_get("rcs").unwrap_or(true) {
                if let Some(ref text) = preloaded[idx].1 {
                    source_from_memory(executor, &preloaded[idx].0, text);
                }
            }
            // suppress unused assignment warning
            let _ = idx;
        }
    }
}

/// `[shell].startup_config` from `zshrs.toml` — sourced once after
/// dotfiles or `canonical_apply`, unless `-f` / `--no-rcs`.
/// Interactive: runs before compsys PATH indexing and the prompt loop.
fn maybe_source_zshrs_startup_config(executor: &mut ShellExecutor, no_rcs: bool) {
    if no_rcs {
        return;
    }
    let Some(path) = zsh::daemon_presence::startup_config_path() else {
        return;
    };
    if !path.is_file() {
        tracing::warn!(
            path = %path.display(),
            "zshrs.toml [shell].startup_config: not a regular file"
        );
        return;
    }
    let Ok(contents) = std::fs::read_to_string(&path) else {
        tracing::warn!(
            path = %path.display(),
            "zshrs.toml [shell].startup_config: read failed"
        );
        return;
    };
    source_from_memory(executor, path.as_path(), &contents);
}

/// Execute a startup file from pre-read memory contents.
/// Mirrors source_file() logic but skips the fs::read_to_string.
fn source_from_memory(executor: &mut ShellExecutor, path: &Path, contents: &str) {
    tracing::trace!(path = %path.display(), "sourcing startup file from memory");

    // Port of `bin_dot()` argzero-save/restore from
    // Src/builtin.c:6076-6079 + 6139-6142: when FUNCTION_ARGZERO
    // is set (default), `$0` becomes the sourced file path during
    // the source and is restored afterwards. The C source uses
    // ztrdup(arg0) to copy and zsfree on exit; Rust's String
    // ownership handles both automatically.
    let saved_argzero = if zsh::ported::options::opt_state_get("functionargzero").unwrap_or(true) {
        let prev = executor.scalar("0");
        executor.set_scalar("0".to_string(), path.to_string_lossy().to_string());
        Some(prev)
    } else {
        None
    };
    // Port of `source()` scriptname swap from Src/init.c:1591-1592
    // (`scriptname = s; scriptfilename = s;`). Drives `%N` / `%x`
    // prompt expansion (and the corresponding xtrace prefix line)
    // to show the sourced file path during the source body, then
    // restore the prior value on exit. The C source pairs this
    // with the argzero swap above — both are in flight together.
    //
    // Two stores per side: the canonical file-static
    // `crate::ported::utils::{scriptname,scriptfilename}` that the
    // prompt expander reads (prompt.rs:152-164 hydrates
    // SCRIPTNAME/SCRIPTFILENAME TLS from these), AND the
    // ShellExecutor.scriptname field used by some legacy callers.
    // Previously only the latter was written, so PS4's `%x` / `%N`
    // kept showing "zsh" (the initial set_scriptname value seeded
    // at executor construction) all the way through .zshenv /
    // .zshrc / .zprofile sourcing. Bug surfaced as
    //   `zsh    zsh    1    export …`
    // (zshrs) vs
    //   `/Users/wizard/.zshenv    /Users/wizard/.zshenv    1    export …`
    // (real zsh) in the user's PS4 parity screenshot.
    let path_str = path.to_string_lossy().to_string();
    let saved_scriptname_field = executor.scriptname.take();
    executor.scriptname = Some(path_str.clone());
    let saved_scriptname = zsh::ported::utils::scriptname_get();
    let saved_scriptfilename = zsh::ported::utils::scriptfilename_get();
    zsh::ported::utils::set_scriptname(Some(path_str.clone()));
    zsh::ported::utils::set_scriptfilename(Some(path_str));

    // Parse and execute the entire file as one stream — port of
    // C `source()` → `loop(0, 0)` (Src/init.c:1551, 1627). The
    // parser handles multi-line constructs (`if/then/fi`,
    // `case/esac`, `for/done`, `while/done`, `function {…}`,
    // heredocs, line continuations) natively because it reads the
    // full token stream rather than discrete lines. The previous
    // line-by-line implementation here split `if [[ -r FILE ]]; then`
    // from its body, so the body executed unconditionally — broke
    // /etc/zshrc:25 zkbd test, broke `case ARM) c1='…'; …;;` SQ
    // assignments, broke any function defined across lines.
    if let Err(e) = executor.execute_script(contents) {
        if e != "__SILENCED__" {
            eprintln!("zshrs: {}: {}", path.display(), e);
        }
    }

    // Restore `$0` per Src/builtin.c:6139-6142.
    if let Some(prev) = saved_argzero {
        match prev {
            Some(v) => {
                executor.set_scalar("0".to_string(), v);
            }
            None => {
                executor.unset_scalar("0");
            }
        }
    }
    // Restore scriptname per Src/init.c source() exit path.
    executor.scriptname = saved_scriptname_field;
    zsh::ported::utils::set_scriptname(saved_scriptname);
    zsh::ported::utils::set_scriptfilename(saved_scriptfilename);
    // c:Src/init.c:1663 — `errflag &= ~ERRFLAG_ERROR;` on source()'s
    // restore path. Sourcing is a containment boundary: an errflag
    // abort inside one startup file must not poison the next file
    // (or the interactive session) — zsh clears the flag when the
    // sourced file unwinds.
    zsh::ported::utils::errflag.fetch_and(
        !zsh::ported::zsh_h::ERRFLAG_ERROR,
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Source a file
fn source_file_with_zwc(executor: &mut ShellExecutor, path: &PathBuf) {
    source_file(executor, path);
}

/// Source a single file, handling multi-line constructs
fn source_file(executor: &mut ShellExecutor, path: &PathBuf) {
    if !path.exists() {
        return;
    }

    if let Ok(contents) = std::fs::read_to_string(path) {
        // c:Src/init.c:1591-1592 — `scriptname = s; scriptfilename
        // = s;` for the duration of the source body. Without these
        // writes, PS4's %x / %N renders "zsh" instead of the file
        // path. See source_from_memory for the longer comment.
        let path_str = path.to_string_lossy().to_string();
        let saved_scriptname = zsh::ported::utils::scriptname_get();
        let saved_scriptfilename = zsh::ported::utils::scriptfilename_get();
        zsh::ported::utils::set_scriptname(Some(path_str.clone()));
        zsh::ported::utils::set_scriptfilename(Some(path_str));
        let mut buffer = String::new();
        let mut in_multiline = false;

        for line in contents.lines() {
            let trimmed = line.trim();

            // Skip empty lines and comments (unless in multiline)
            if !in_multiline && (trimmed.is_empty() || trimmed.starts_with('#')) {
                continue;
            }

            // Check for line continuation
            if let Some(stripped) = line.strip_suffix('\\') {
                buffer.push_str(stripped);
                buffer.push(' ');
                in_multiline = true;
                continue;
            }

            // Check for unclosed constructs (heredoc, quotes, braces)
            if in_multiline {
                buffer.push_str(line);
                // Simple heuristic: if we have balanced braces/quotes, execute
                let open_braces = buffer.matches('{').count();
                let close_braces = buffer.matches('}').count();
                let open_parens = buffer.matches('(').count();
                let close_parens = buffer.matches(')').count();

                if open_braces == close_braces && open_parens == close_parens {
                    process_line(&buffer, executor);
                    buffer.clear();
                    in_multiline = false;
                } else {
                    buffer.push('\n');
                }
            } else {
                process_line(line, executor);
            }
        }

        // Process any remaining buffered content
        if !buffer.is_empty() {
            process_line(&buffer, executor);
        }
        // Restore scriptname/scriptfilename per Src/init.c source()
        // exit path. Mirrors the equivalent restore in
        // source_from_memory above.
        zsh::ported::utils::set_scriptname(saved_scriptname);
        zsh::ported::utils::set_scriptfilename(saved_scriptfilename);
    }
}

/// Source logout files when shell exits (per zshall(1))
/// Only for login shells, respects RCS and GLOBAL_RCS options
#[allow(dead_code)]
fn source_logout_files(executor: &mut ShellExecutor, is_login: bool) {
    if !is_login {
        return;
    }

    // Check RCS option
    if !zsh::ported::options::opt_state_get("rcs").unwrap_or(true) {
        return;
    }

    let zdotdir = get_zdotdir();

    // $ZDOTDIR/.zlogout first
    source_file_with_zwc(executor, &zdotdir.join(".zlogout"));

    // /etc/zlogout (only if GLOBAL_RCS is set)
    if zsh::ported::options::opt_state_get("globalrcs").unwrap_or(true) {
        source_file_with_zwc(executor, &global_rc("zlogout"));
    }
}

fn process_line(line: &str, executor: &mut ShellExecutor) {
    // @ prefix: dispatch to stryke if fat binary registered a handler
    if line.starts_with('@') {
        let code = line.trim_start_matches('@').trim();
        if !code.is_empty() {
            if let Some(status) = zsh::try_stryke_dispatch(code) {
                executor.set_last_status(status);
                return;
            }
            // No handler registered (thin binary) — treat @ as normal shell input
        }
    }

    if let Err(e) = executor.execute_script(line) {
        if e != "__SILENCED__" {
            eprintln!("zshrs: {}", e);
        }
    }
}
