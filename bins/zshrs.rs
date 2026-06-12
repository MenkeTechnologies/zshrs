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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use nu_ansi_term::{Color as AnsiColor, Style as AnsiStyle};
use reedline::Color;
use reedline::{
    default_emacs_keybindings, menu_functions, Completer, DefaultHinter, Editor, Emacs,
    FileBackedHistory, Highlighter, KeyCode, KeyModifiers, Menu as ReedlineMenuTrait, MenuBuilder,
    MenuEvent, MenuSettings, Painter, Prompt, PromptHistorySearch, PromptHistorySearchStatus,
    Reedline, ReedlineEvent, ReedlineMenu, Signal, Span, StyledText, Suggestion, ValidationResult,
    Validator,
};

use zsh::history::HistoryEngine;
use zsh::vm_helper::ShellExecutor;
use zsh::zwc;

use zsh::compsys::{build_cache_from_fpath, cache::CompsysCache, compinit_lazy, get_system_fpath};
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
#[derive(Clone, Debug, Default)]
struct MenuState;
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
#[derive(Clone, Debug, Default)]
struct MenuRendering {
    lines: Vec<MenuLine>,
}
#[derive(Clone, Debug, Default)]
struct MenuLine {
    content: String,
}
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

use zsh::{highlight_shell, validate_command, HighlightRole, ValidationStatus};

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
  --dap HOST:PORT        DAP debug adapter; connects back to the IDE's listener
                         at HOST:PORT and serves the Debug Adapter Protocol
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
  --sh         identical-behaviour drop-in for /bin/sh / POSIX (alias of --posix)
  --csh        identical-behaviour drop-in for /bin/csh
  --posix      identical-behaviour drop-in for /bin/sh (Bourne / dash)
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
    /// `/bin/sh` (Bourne / dash).
    Posix,
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
/// `is_zshrs_mode` — see implementation.
pub fn is_zshrs_mode() -> bool {
    matches!(shell_mode(), ShellMode::Zshrs)
}

/// Legacy compat shim — maps to --zsh mode
pub fn is_zsh_compat() -> bool {
    is_zsh_mode()
}

fn main() {
    // c:Src/params.c:893 createparamtable reads `environ` exactly as
    // it was at process entry. Snapshot it as the first statement so
    // nothing later in shell init (setenv from builtins, lazy crate
    // init) skews the import.
    //
    // Known unfixable artifact: zshrs links CoreFoundation through a
    // dependency, and CF's dyld initializer runs BEFORE main and may
    // rewrite __CF_USER_TEXT_ENCODING in the live environment (zsh
    // has no such initializer, so it imports the original). The
    // kernel's exec-image copy (sysctl KERN_PROCARGS2) was tried and
    // REJECTED: it silently truncates large environments (tail vars
    // vanish), which corrupts far more than the one CF variable.
    let _ = zsh::ported::params::environ.set(std::env::vars().collect());
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
    zshrs_main();
}

/// Main entry point — extracted so the fat binary can call it after
/// registering the stryke handler.
pub fn zshrs_main() {
    // The `--daemon` flag is gone — the daemon is its own binary
    // (`zshrs-daemon`), never invoked through the shell. Catch the
    // legacy invocation and point users at the right install path
    // instead of silently mis-parsing it.
    {
        let early_args: Vec<String> = env::args().collect();
        if early_args.iter().any(|a| a == "--daemon") {
            eprintln!(
                "zshrs: `--daemon` is no longer supported.\n\
                 The daemon is a separate binary; install + run via one of:\n  \
                   zshrs-daemon                                  # foreground manual run\n  \
                   systemctl --user enable --now zshrs-daemon    # Linux (per-user systemd)\n  \
                   launchctl load ~/Library/LaunchAgents/...     # macOS (see examples/install-launchd.sh)\n  \
                   brew services start zshrs                     # macOS / Linux brew\n\
                 See examples/{{systemd,launchd,brew}}/ for unit files."
            );
            std::process::exit(2);
        }
    }

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
        let raw: Vec<String> = env::args().collect();
        let mut out: Vec<String> = Vec::with_capacity(raw.len());
        for a in &raw {
            let bytes = a.as_bytes();
            let is_clumped = bytes.len() >= 3
                && bytes[0] == b'-'
                && bytes[1] != b'-'
                && bytes.iter().skip(1).all(|c| c.is_ascii_alphabetic());
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
        "sh" | "dash" => Some(ShellMode::Posix),
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
    let explicit_mode: Option<ShellMode> = if args.iter().any(|a| a == "--posix" || a == "--sh") {
        Some(ShellMode::Posix)
    } else if args.iter().any(|a| a == "--bash") {
        Some(ShellMode::Bash)
    } else if args.iter().any(|a| a == "--ksh") {
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
            Some("ksh") => Some(ShellMode::Ksh),
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
    let emu_name = match shell_mode() {
        ShellMode::Ksh => "ksh",
        ShellMode::Posix => "sh",
        ShellMode::Bash => "sh", // bash ≈ sh emulation; bash-specific bits flagged via is_bash_mode()
        ShellMode::Zsh | ShellMode::Zshrs => "zsh",
    };
    if argv0_basename == "csh" || args.iter().any(|a| a == "--csh") {
        zsh::ported::options::emulate("csh", true);
    } else {
        zsh::ported::options::emulate(emu_name, true);
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

    // --dap HOST:PORT: connect back to the IntelliJ DAP client at the
    // given address and serve the Debug Adapter Protocol. Implementation
    // in src/extensions/dap.rs.
    if let Some(i) = args.iter().position(|a| a == "--dap") {
        let addr = args.get(i + 1).map(|s| s.as_str()).unwrap_or("127.0.0.1:0");
        std::process::exit(zsh::dap::run_dap(addr));
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

    // Extract flags before filtering: -x (xtrace), -f (no rcs), -v (verbose)
    let enable_xtrace = args.iter().any(|a| a == "-x");
    let enable_verbose = args.iter().any(|a| a == "-v");
    // -f / --no-rcs: skip startup files AND turn off rcs + hashdirs.
    // zsh's `-f`-mode `setopt` lists `nohashdirs` and `norcs` for this
    // reason; without these inserts, zshrs's `setopt` reported an
    // empty list under `-f`.
    let no_rcs_flag = args.iter().any(|a| a == "-f" || a == "--no-rcs");

    // Collect `-o NAME` (set option) and `+o NAME` (unset option)
    // pairs from the CLI before filtering. Direct port of zsh's
    // option-on-command-line behavior — `zsh -f +o nomatch -c '...'`
    // disables nomatch for the run. Without parsing, `+o` was taken
    // as a script file argument and zshrs errored.
    let mut option_settings: Vec<(String, bool)> = Vec::new();
    {
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if (a == "-o" || a == "+o") && i + 1 < args.len() {
                let setval = a == "-o";
                option_settings.push((args[i + 1].clone(), setval));
                i += 2;
            } else {
                i += 1;
            }
        }
    }

    // Filter out flags that don't affect -c / script dispatch and reject
    // unknown long options so typos (e.g. `--poop`, `--dump-wordgcode`)
    // fail loudly instead of falling through to interactive shell startup.
    //
    // The filter must enumerate every long flag the binary recognizes —
    // either to consume it here, or to pass it through for downstream
    // detection. Long flags handled by earlier `return`-on-match blocks
    // (--help/--version/--doctor/--daemon/--dump-*) never reach this loop.
    // After `--`, all remaining tokens are positional.
    let args: Vec<String> = {
        let mut out: Vec<String> = Vec::new();
        let mut i = 0;
        let mut saw_dashdash = false;
        while i < args.len() {
            let a = &args[i];
            if saw_dashdash {
                out.push(a.clone());
                i += 1;
                continue;
            }
            if a == "--" {
                saw_dashdash = true;
                out.push(a.clone());
                i += 1;
                continue;
            }
            // Long flags consumed here: don't propagate to downstream
            // -c / script dispatch (their effect is captured earlier).
            if a == "--zsh-compat"
                || a == "--zsh"
                || a == "--bash"
                || a == "--ksh"
                || a == "--sh"
                || a == "--csh"
                || a == "--posix"
                || a == "-f"
                || a == "--no-rcs"
                || a == "-x"
                || a == "-v"
                || a == "--disasm"
            {
                i += 1;
                continue;
            }
            // `--emulate MODE` — consume the flag AND the next arg.
            if a == "--emulate" && i + 1 < args.len() {
                i += 2;
                continue;
            }
            if (a == "-o" || a == "+o") && i + 1 < args.len() {
                // Consume the next arg as the option name and skip
                // both — already captured above.
                i += 2;
                continue;
            }
            // Long flags passed through for later detection: --login is
            // checked downstream at the is_login site; --xtrace / --verbose
            // are checked at the argv-scan sites.
            if a == "--login" || a == "--xtrace" || a == "--verbose" {
                out.push(a.clone());
                i += 1;
                continue;
            }
            // Any remaining `--*` is unknown. C zsh emits
            // `zsh: no such option: <name>` (no leading dashes); match that.
            if let Some(name) = a.strip_prefix("--") {
                eprintln!("zshrs: no such option: {}", name);
                std::process::exit(1);
            }
            out.push(a.clone());
            i += 1;
        }
        out
    };

    /// Apply CLI flags and shell mode to executor
    fn apply_cli_flags(
        executor: &mut ShellExecutor,
        xtrace: bool,
        verbose: bool,
        no_rcs: bool,
        opts: &[(String, bool)],
    ) {
        // Apply shell mode
        executor.zsh_compat = is_zsh_mode();
        executor.bash_compat = is_bash_mode();
        if is_posix_mode() {
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
            // Match zsh -f: rcs and hashdirs default-on options are
            // turned off so `setopt` lists `nohashdirs norcs`. zsh
            // keeps globalrcs on (only the user-rcs files are skipped).
            zsh::ported::options::opt_state_set("rcs", false);
            zsh::ported::options::opt_state_set("hashdirs", false);
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
        let stdin_isatty = unsafe { libc::isatty(0) != 0 };
        if !stdin_isatty {
            zsh::ported::options::opt_state_set("monitor", false);
            zsh::ported::options::opt_state_set("hashdirs", false);
        }
        // Apply CLI `-o NAME` / `+o NAME` option settings.
        for (raw, set_val) in opts {
            let canonical = raw.to_lowercase().replace(['_', '-'], "");
            zsh::ported::options::opt_state_set(&canonical, *set_val);
        }
    }

    // Handle -c 'command' syntax
    if args.len() >= 3 && args[1] == "-c" {
        let code = &args[2];

        let mut executor = ShellExecutor::new();
        apply_cli_flags(
            &mut executor,
            enable_xtrace,
            enable_verbose,
            no_rcs_flag,
            &option_settings,
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
        // Route through `push_zsh_eval_context` so the tied array
        // `zsh_eval_context[*]` is populated too — `${zsh_eval_context[*]}`
        // expansion reads the array, not the scalar. Bug #262 in
        // docs/BUGS.md. The push owns both the static C-port stack
        // AND the paramtab mirror.
        zsh::vm_helper::push_zsh_eval_context("cmdarg");
        // POSIX `sh -c script [name [args...]]` semantics
        // (Src/init.c:271 + 479): the next non-option arg AFTER the
        // command string becomes $0; remaining args become $1, $2, …
        // When no name is supplied, $0 falls back to argv[0] (the
        // binary path) — matching `zsh -c '...'`'s behavior of
        // exposing the full path of the shell binary.
        let zero = if args.len() > 3 {
            // `zshrs -c 'cmd' name args...` — args layout after the
            // --zsh / -f / -x filter at line 684 is:
            //   args[0] = binary path
            //   args[1] = "-c"
            //   args[2] = the command string
            //   args[3] = $0 name
            //   args[4..] = $1, $2, …
            executor.set_pparams(args[4..].to_vec());
            args[3].clone()
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

    // Handle script file argument
    if args.len() >= 2 && !args[1].starts_with('-') {
        let mut executor = ShellExecutor::new();
        apply_cli_flags(
            &mut executor,
            enable_xtrace,
            enable_verbose,
            no_rcs_flag,
            &option_settings,
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

    // Check if stdin is a TTY
    if atty::is(atty::Stream::Stdin) {
        run_interactive();
    } else {
        run_non_interactive();
    }
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
        ("/etc/zshenv", true),
        (&format!("{}/.zshenv", zdotdir), false),
        ("/etc/zprofile", false),
        (&format!("{}/.zprofile", zdotdir), false),
        ("/etc/zshrc", false),
        (&format!("{}/.zshrc", zdotdir), false),
        ("/etc/zlogin", false),
        (&format!("{}/.zlogin", zdotdir), false),
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

fn run_non_interactive() {
    let mut executor = ShellExecutor::new();
    executor.zsh_compat = is_zsh_mode();
    executor.bash_compat = is_bash_mode();
    if is_posix_mode() {
        executor.enter_posix_mode();
    }
    if is_ksh_mode() {
        executor.enter_ksh_mode();
    }
    // Apply -x / -v from argv. Same wiring as the `-c` and
    // script-file paths — without this, `cmd | zshrs -x` (stdin
    // pipe, no -c, no script) silently runs without xtrace because
    // stdin-not-tty bypasses run_interactive.
    let argv: Vec<String> = std::env::args().collect();
    if argv.iter().any(|a| a == "-x" || a == "--xtrace") {
        zsh::ported::options::opt_state_set("xtrace", true);
    }
    if argv.iter().any(|a| a == "-v" || a == "--verbose") {
        zsh::ported::options::opt_state_set("verbose", true);
    }
    // Apply CLI `-o NAME` / `+o NAME` option settings — same
    // normalization as apply_cli_flags. Without this, `zshrs -f -o
    // CONTINUE_ON_ERROR <<< script` silently dropped the option
    // (only the -c and script-file paths parsed -o).
    {
        let mut i = 0;
        while i < argv.len() {
            let a = &argv[i];
            if (a == "-o" || a == "+o") && i + 1 < argv.len() {
                let canonical = argv[i + 1].to_lowercase().replace(['_', '-'], "");
                zsh::ported::options::opt_state_set(&canonical, a == "-o");
                i += 2;
            } else {
                i += 1;
            }
        }
    }
    // c:Src/init.c:307-308 — `} else if (!*cmdptr) opts[SHINSTDIN] = 1;`
    // No script-file argument and no -c command means the shell reads
    // commands from stdin, and SHINSTDIN must be set. Diagnostics key
    // off it: zwarning (Src/utils.c:114 + 301) prints `zsh: msg` with
    // NO line number when SHINSTDIN is set at top level, vs
    // `name:LINE: msg` for -c/script input.
    zsh::ported::options::opt_state_set("shinstdin", true);
    // Read all of stdin at once so multi-line constructs (heredocs, functions,
    // loops, etc.) are parsed correctly — line-by-line breaks them.
    let mut script = String::new();
    io::stdin().lock().read_to_string(&mut script).unwrap_or(0);
    if !script.is_empty() {
        if let Err(e) = executor.execute_script(&script) {
            if e != "__SILENCED__" {
                eprintln!("zshrs: {}", e);
            }
            std::process::exit(1);
        }
        std::process::exit(executor.last_status());
    }
}

fn get_zdotdir() -> PathBuf {
    std::env::var("ZDOTDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
}

/// Source zsh startup files in correct order per zshall(1) STARTUP/SHUTDOWN FILES
///
/// Behavior is controlled by RCS and GLOBAL_RCS options:
/// - RCS (default: on) - if unset, no startup files are read
/// - GLOBAL_RCS (default: on) - if unset, /etc/* files are skipped
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
    candidates.push(PathBuf::from("/etc/zshenv"));

    if !no_rcs {
        // Phase 1: user .zshenv
        candidates.push(zdotdir.join(".zshenv"));

        // Phase 2: login profile files
        if is_login {
            candidates.push(PathBuf::from("/etc/zprofile"));
            candidates.push(zdotdir.join(".zprofile"));
        }

        // Phase 3: interactive rc files
        if is_interactive {
            candidates.push(PathBuf::from("/etc/zshrc"));
            candidates.push(zdotdir.join(".zshrc"));
        }

        // Phase 4: login files (after zshrc)
        if is_login {
            candidates.push(PathBuf::from("/etc/zlogin"));
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
/// Interactive: runs before compsys PATH indexing, reedline setup, and
/// the `read_line` prompt loop.
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
        source_file_with_zwc(executor, &PathBuf::from("/etc/zlogout"));
    }
}

/// Drain stale terminal-response bytes from stdin before re-entering
/// the line editor. Mirrors the effective behavior of C ZLE's
/// byte-input loop: `read(SHTTY, cptr, 1)` at Src/Zle/zle_main.c:838
/// (called via `getbyte`/`getfullchar` at Src/Zle/zle_main.c:967) reads
/// one byte at a time and the keymap dispatch silently consumes
/// escape sequences that have no widget binding — most relevantly
/// `\e[<row>;<col>R` Cursor Position Report replies left in stdin by
/// alt-screen programs like `less` after they query cursor state.
///
/// reedline's `crossterm::cursor::position()` issues a fresh DSR
/// (`\e[6n`) and parses the response synchronously; if stale CPR bytes
/// from the previous foreground program are still buffered it either
/// reads the wrong reply or times out with "cursor position could not
/// be read within a normal duration", and the leftover `^[[N;NR` then
/// leaks onto the next prompt. Polling with timeout=0 + `read(2)`
/// drops those stale bytes the same way C ZLE's input loop would —
/// the keymap has no binding for `\e[<n>;<n>R`, so they are consumed
/// without effect.
fn drain_stale_terminal_input() {
    use std::os::unix::io::AsRawFd;
    let fd = std::io::stdin().as_raw_fd();
    let mut buf = [0u8; 256];
    let mut total = 0usize;
    // Bound the loop so a misbehaving terminal can't pin us forever.
    for _ in 0..32 {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let r = unsafe { libc::poll(&mut pfd, 1, 0) };
        if r <= 0 || (pfd.revents & libc::POLLIN) == 0 {
            break;
        }
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
        total += n as usize;
    }
    if total > 0 {
        tracing::debug!(bytes = total, "drained stale terminal input pre-prompt");
    }
}

fn run_interactive() {
    tracing::info!("interactive mode starting");
    // Set up signal handling
    let interrupted = Arc::new(AtomicBool::new(false));
    let i = interrupted.clone();
    ctrlc::set_handler(move || {
        i.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Executor + RC chain + `[shell].startup_config` run before compsys
    // PATH indexing, reedline, and the read_line prompt loop so env
    // from init scripts is visible to completion DB and the first prompt.
    let mut executor = ShellExecutor::new();
    executor.zsh_compat = is_zsh_mode();
    executor.bash_compat = is_bash_mode();
    if is_posix_mode() {
        executor.enter_posix_mode();
    }
    if is_ksh_mode() {
        executor.enter_ksh_mode();
    }

    // Determine shell type from invocation per zshall(1)
    let args: Vec<String> = std::env::args().collect();

    // -f: don't source startup files (except /etc/zshenv which is ALWAYS read)
    let no_rcs = args.iter().any(|a| a == "-f" || a == "--no-rcs");

    // -x / --xtrace: print each command before executing. zsh's
    // `setopt XTRACE` is wired identically across `-c`, script-file,
    // and interactive modes; the previous interactive-only branch
    // skipped this flag so `zshrs -x` (no -c, no script) silently
    // ran without xtrace. Mirror what apply_cli_flags does in the
    // other modes — set the option BEFORE source_startup_files so
    // every line of `.zshenv` / `.zshrc` is also traced, matching
    // `zsh -x` (which sets XTRACE before init scripts run).
    if args.iter().any(|a| a == "-x" || a == "--xtrace") {
        zsh::ported::options::opt_state_set("xtrace", true);
    }
    if args.iter().any(|a| a == "-v" || a == "--verbose") {
        zsh::ported::options::opt_state_set("verbose", true);
    }

    // Login shell detection:
    // - explicit -l or --login flag
    // - invoked as -zshrs (name starts with -)
    // - $SHELL ends with zshrs (login shell)
    let is_login = args.iter().any(|a| a == "-l" || a == "--login")
        || args.first().map(|a| a.starts_with('-')).unwrap_or(false)
        || std::env::var("SHELL")
            .map(|s| s.ends_with("zshrs"))
            .unwrap_or(false);

    let is_interactive = true; // We're in run_interactive()

    // Set default options (RCS and GLOBAL_RCS are on by default)
    zsh::ported::options::opt_state_set("rcs", true);
    zsh::ported::options::opt_state_set("globalrcs", true);

    // Source startup files in correct zsh order per zshall(1).
    // OR — if the daemon is up and serving zshrs canonical state, AND
    // the user opted in via `[shell] skip_configs`, skip every dotfile
    // (including /etc/zshenv) and apply canonical state from the
    // daemon instead. This is the ~10ms cold-start path: no parse,
    // no .zshrc evaluation, no plugin discovery.
    #[cfg(feature = "daemon")]
    {
        if zsh::daemon_presence::should_skip_configs() {
            let applied = zsh::canonical_apply::apply_all(&mut executor);
            tracing::info!(
                rows = applied,
                "skip_configs: dotfile chain bypassed, canonical state applied from daemon"
            );
        } else {
            source_startup_files(&mut executor, is_login, is_interactive, no_rcs);
        }
    }
    #[cfg(not(feature = "daemon"))]
    source_startup_files(&mut executor, is_login, is_interactive, no_rcs);

    maybe_source_zshrs_startup_config(&mut executor, no_rcs);

    // Initialize compsys SQLite mirror (read-only inspection target;
    // the authoritative completion cache is the rkyv-mmap'd shard set)
    let cache_path = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("zshrs/compsys.db");
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let compsys_cache = match CompsysCache::open(&cache_path) {
        Ok(mut cache) => {
            // Index PATH executables on first run. This is one-time startup
            // work — log it via tracing instead of printing to stderr where it
            // pollutes user terminals (and tests, which see stderr noise).
            if !cache.has_executables().unwrap_or(false) {
                let path_var = std::env::var("PATH").unwrap_or_default();
                let started = std::time::Instant::now();
                let mut executables = Vec::new();
                for dir in path_var.split(':') {
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        for entry in entries.flatten() {
                            if let Ok(ft) = entry.file_type() {
                                if ft.is_file() || ft.is_symlink() {
                                    if let Some(name) = entry.file_name().to_str() {
                                        executables.push((name.to_string(), dir.to_string()));
                                    }
                                }
                            }
                        }
                    }
                }
                let cmd_count = executables.len();
                let _ = cache.set_executables_bulk(&executables);
                tracing::info!(
                    commands = cmd_count,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "indexed PATH completions"
                );
            }
            Some(cache)
        }
        Err(e) => {
            tracing::warn!(error = %e, "compsys cache failed to initialize");
            None
        }
    };

    // Initialize SQLite history engine for frequency tracking
    let history_engine: Option<std::sync::Arc<std::sync::Mutex<HistoryEngine>>> =
        match HistoryEngine::new() {
            Ok(engine) => {
                let count = engine.count().unwrap_or(0);
                if count > 0 {
                    tracing::info!(entries = count, "history loaded");
                }
                Some(std::sync::Arc::new(std::sync::Mutex::new(engine)))
            }
            Err(e) => {
                tracing::warn!(error = %e, "history engine failed to initialize");
                None
            }
        };

    let line_editor = setup_editor(compsys_cache.map(|c| (c, cache_path)));
    if line_editor.is_none() {
        eprintln!("Failed to initialize line editor");
        return;
    }
    let mut line_editor = line_editor.unwrap();

    // Banner goes to the log, not the user's terminal. A shell prompt should
    // appear immediately on launch — no version stripe, no "type exit" hint.
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "interactive shell start"
    );

    loop {
        // Non-blocking: merge background compinit results if ready
        executor.drain_compinit_bg();

        let prompt = ZshrsPrompt::new(&executor);
        // Drop stale terminal-response bytes (CPR replies, mode-reset
        // acks, etc.) left over from the previous foreground program
        // before reedline issues its own cursor-position query. See
        // `drain_stale_terminal_input` doc — Src/Zle/zle_main.c:838+967.
        drain_stale_terminal_input();
        match line_editor.read_line(&prompt) {
            Ok(Signal::Success(line)) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                if line == "exit" || line == "logout" {
                    break;
                }

                // Long-cmd-started watchdog: spawn a thread that fires
                // cmd_started IPC if the command runs longer than the
                // threshold without completing. Per docs/DAEMON.md
                // "Long-running command completion notices — companion
                // events long_cmd_started fires when a command crosses 5s
                // of runtime (not waiting for completion)".
                #[cfg(feature = "daemon")]
                let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                #[cfg(feature = "daemon")]
                {
                    let line_owned = line.to_string();
                    let cwd_owned = std::env::current_dir()
                        .ok()
                        .map(|p| p.to_string_lossy().to_string());
                    let completed = std::sync::Arc::clone(&completed);
                    let threshold_secs: u64 = std::env::var("ZSHRS_LONG_CMD_PRE_THRESHOLD")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(5);
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(threshold_secs));
                        if !completed.load(std::sync::atomic::Ordering::SeqCst) {
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
                process_line(line, &mut executor);
                #[cfg(feature = "daemon")]
                completed.store(true, std::sync::atomic::Ordering::SeqCst);
                let duration_ns_total = start.elapsed().as_nanos() as i64;
                let duration = duration_ns_total / 1_000_000;

                // Ship history write to worker pool — prompt returns instantly,
                // SQLite write happens in background.
                if let Some(ref engine) = history_engine {
                    let engine = std::sync::Arc::clone(engine);
                    let line = line.to_string();
                    let cwd = std::env::current_dir()
                        .ok()
                        .map(|p| p.to_string_lossy().to_string());
                    let status = executor.last_status();
                    executor.worker_pool.submit(move || {
                        if let Ok(eng) = engine.lock() {
                            if let Ok(id) = eng.add(&line, cwd.as_deref()) {
                                let _ = eng.update_last(id, duration, status);
                            }
                        }
                    });
                }

                // Daemon history_append IPC — daemon stores the row in
                // history.db, broadcasts long_cmd_complete / failed /
                // signaled to other shells if duration > threshold.
                #[cfg(feature = "daemon")]
                {
                    let line_owned = line.to_string();
                    let cwd_owned = std::env::current_dir()
                        .ok()
                        .map(|p| p.to_string_lossy().to_string());
                    let status = executor.last_status() as i64;
                    executor.worker_pool.submit(move || {
                        let _ = zsh::daemon::client::call_once_no_spawn(
                            "history_append",
                            serde_json::json!({
                                "line": line_owned,
                                "exit_code": status,
                                "cwd": cwd_owned,
                                "duration_ns": duration_ns_total,
                            }),
                        );
                    });
                }
            }
            Ok(Signal::CtrlD) => {
                // EOF - exit shell
                executor.run_trap("EXIT");
                println!();
                break;
            }
            Ok(Signal::CtrlC) => {
                // Interrupt - run INT trap if set, otherwise just print newline
                interrupted.store(false, Ordering::SeqCst);
                executor.run_trap("INT");
                println!();
                continue;
            }
            Ok(_) => {
                // Handle any other signals
                continue;
            }
            Err(err) => {
                eprintln!("Error: {err}");
                break;
            }
        }
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

fn setup_editor(compsys_cache: Option<(CompsysCache, PathBuf)>) -> Option<Reedline> {
    // Single-directory rule: the reedline history file lives inside
    // $ZSHRS_HOME / ~/.zshrs alongside the sqlite index, NOT at
    // ~/.zshrs_history. `HistoryEngine::text_path` is the single
    // source of truth so renames stay coherent.
    let history_path = HistoryEngine::text_path();
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let history = Box::new(FileBackedHistory::with_file(10000, history_path).ok()?);

    let mut keybindings = default_emacs_keybindings();

    // Add Tab keybinding to trigger completion menu
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );

    // Zsh-style daily-driver bindings missing from reedline's emacs default.
    // Each cites the matching default in Src/Zle/zle_bindings.c (the emacs
    // bindkey table) so the lineage is traceable.
    //
    // Ctrl-_ → undo. zle_bindings.c maps `^_` to `z_undo` in the emacs
    // table. Reedline's default uses Ctrl-z for undo, but Ctrl-_ is the
    // muscle-memory key for zsh users.
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('_'),
        ReedlineEvent::Edit(vec![reedline::EditCommand::Undo]),
    );
    // Ctrl-r → reverse-incremental-search-history. zle_bindings.c maps
    // `^R` to `z_historyincrementalsearchbackward`.
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('r'),
        ReedlineEvent::SearchHistory,
    );
    // Ctrl-s — forward isearch. zle_bindings.c → z_historyincrementalsearchforward.
    // Reedline only has SearchHistory (one direction), so we still surface
    // the menu — useful even without the C two-direction split.
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('s'),
        ReedlineEvent::SearchHistory,
    );
    // Alt-. → insert-last-word. zle_bindings.c maps `\e.` to
    // `z_insertlastword`. Reedline lacks a literal "insert last argument
    // from prior history", but HistoryHintComplete is the closest behavior
    // (accepts the suggested completion of a prior history match).
    keybindings.add_binding(
        KeyModifiers::ALT,
        KeyCode::Char('.'),
        ReedlineEvent::HistoryHintComplete,
    );

    let edit_mode = Box::new(Emacs::new(keybindings));

    let mut editor = Reedline::create()
        .with_history(history)
        .with_edit_mode(edit_mode)
        .with_hinter(Box::new(
            DefaultHinter::default().with_style(AnsiStyle::new().fg(AnsiColor::DarkGray)),
        ))
        .with_highlighter(Box::new(ZshrsHighlighter))
        .with_validator(Box::new(ZshrsValidator));

    if let Some((cache, cache_path)) = compsys_cache {
        let completer = Box::new(ZshrsCompleter::new(cache, cache_path));

        // Zsh-style menuselect — port of Src/Zle/complist.c domenuselect()
        // Uses compsys MenuState for rendering (group headers, zstyle colors,
        // grid navigation with column memory, viewport scrolling).
        let completion_menu = Box::new(ZshMenuSelect::new());

        editor = editor
            .with_completer(completer)
            .with_menu(ReedlineMenu::EngineCompleter(completion_menu));
    }

    Some(editor)
}

// ============================================================================
// SYNTAX HIGHLIGHTER - Fish-style real-time highlighting
// ============================================================================

struct ZshrsHighlighter;

impl Highlighter for ZshrsHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let specs = highlight_shell(line);
        let mut styled = StyledText::new();

        if line.is_empty() {
            return styled;
        }

        let mut current_style = AnsiStyle::new();
        let mut current_text = String::new();
        let mut last_role = HighlightRole::Normal;

        for (i, c) in line.chars().enumerate() {
            let byte_pos = line.char_indices().nth(i).map(|(p, _)| p).unwrap_or(i);
            let role = specs
                .get(byte_pos)
                .map(|s| s.foreground)
                .unwrap_or(HighlightRole::Normal);

            if role != last_role && !current_text.is_empty() {
                styled.push((current_style, current_text.clone()));
                current_text.clear();
            }

            if role != last_role {
                current_style = role_to_style(role);
                last_role = role;
            }

            current_text.push(c);
        }

        if !current_text.is_empty() {
            styled.push((current_style, current_text));
        }

        styled
    }
}

fn role_to_style(role: HighlightRole) -> AnsiStyle {
    match role {
        HighlightRole::Normal => AnsiStyle::new(),
        HighlightRole::Command => AnsiStyle::new().fg(AnsiColor::Green).bold(),
        HighlightRole::Keyword => AnsiStyle::new().fg(AnsiColor::Blue).bold(),
        HighlightRole::Statement => AnsiStyle::new().fg(AnsiColor::Magenta).bold(),
        HighlightRole::Param => AnsiStyle::new(),
        HighlightRole::Option => AnsiStyle::new().fg(AnsiColor::Cyan),
        HighlightRole::Comment => AnsiStyle::new().fg(AnsiColor::DarkGray),
        HighlightRole::Error => AnsiStyle::new().fg(AnsiColor::Red).bold(),
        HighlightRole::String => AnsiStyle::new().fg(AnsiColor::Yellow),
        HighlightRole::Escape => AnsiStyle::new().fg(AnsiColor::Yellow).bold(),
        HighlightRole::Operator => AnsiStyle::new().fg(AnsiColor::White).bold(),
        HighlightRole::Redirection => AnsiStyle::new().fg(AnsiColor::Magenta),
        HighlightRole::Path => AnsiStyle::new().underline(),
        HighlightRole::PathValid => AnsiStyle::new().fg(AnsiColor::Green).underline(),
        HighlightRole::Autosuggestion => AnsiStyle::new().fg(AnsiColor::DarkGray),
        HighlightRole::Selection => AnsiStyle::new().reverse(),
        HighlightRole::Search => AnsiStyle::new().fg(AnsiColor::Black).on(AnsiColor::Yellow),
        HighlightRole::Variable => AnsiStyle::new().fg(AnsiColor::Cyan).bold(),
        HighlightRole::Quote => AnsiStyle::new().fg(AnsiColor::Yellow),
    }
}

// ============================================================================
// VALIDATOR - Multi-line support for incomplete commands
// ============================================================================

struct ZshrsValidator;

impl Validator for ZshrsValidator {
    fn validate(&self, line: &str) -> ValidationResult {
        match validate_command(line) {
            ValidationStatus::Valid => ValidationResult::Complete,
            ValidationStatus::Incomplete => ValidationResult::Incomplete,
            ValidationStatus::Invalid(_) => ValidationResult::Complete, // Let execution show the error
        }
    }
}

// ============================================================================
// ZSH MENUSELECT — Port of Src/Zle/complist.c domenuselect()
// ============================================================================

/// Reedline Menu backed by compsys MenuState.
///
/// Bridges reedline's MenuEvent dispatch to our full zsh-style menuselect
/// with group headers, zstyle colors, grid navigation with column memory,
/// and proper viewport scrolling.
struct ZshMenuSelect {
    settings: MenuSettings,
    active: bool,
    /// compsys menu state — the real engine
    state: MenuState,
    /// Cached reedline suggestions (kept for get_values/replace_in_buffer)
    values: Vec<Suggestion>,
    event: Option<MenuEvent>,
    /// Whether values have been loaded into MenuState
    loaded: bool,
}

impl ZshMenuSelect {
    fn new() -> Self {
        Self {
            settings: MenuSettings::default().with_name("completion_menu"),
            active: false,
            state: MenuState::new(),
            values: Vec::new(),
            event: None,
            loaded: false,
        }
    }

    /// Convert reedline Suggestions to compsys CompletionGroup and load into MenuState
    fn load_suggestions(&mut self, terminal_width: u16) {
        if self.values.is_empty() || self.loaded {
            return;
        }

        // Group suggestions by their extra[0] tag (e.g. "command", "file", "option")
        let mut groups: std::collections::HashMap<String, Vec<CompsysCompletion>> =
            std::collections::HashMap::new();

        for sugg in &self.values {
            let group_name = sugg
                .extra
                .as_ref()
                .and_then(|e| e.first())
                .cloned()
                .unwrap_or_else(|| "completions".to_string());

            let mut comp = CompsysCompletion::new(&sugg.value);
            if let Some(ref desc) = sugg.description {
                comp.desc = Some(desc.clone());
            }
            groups.entry(group_name).or_default().push(comp);
        }

        let mut comp_groups = Vec::new();
        for (name, matches) in groups {
            let mut g = CompletionGroup::new(&name);
            g.matches = matches;
            comp_groups.push(g);
        }

        self.state.set_term_size(terminal_width as usize, 24);
        // set_completions disabled after CompletionGroup stub-out.
        let _ = &comp_groups;
        self.state.start();
        self.loaded = true;
    }

    fn index(&self) -> usize {
        self.state.selected_index().unwrap_or(0)
    }
}

impl MenuBuilder for ZshMenuSelect {
    fn settings_mut(&mut self) -> &mut MenuSettings {
        &mut self.settings
    }
}

impl ReedlineMenuTrait for ZshMenuSelect {
    fn settings(&self) -> &MenuSettings {
        &self.settings
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn can_quick_complete(&self) -> bool {
        true
    }

    fn can_partially_complete(
        &mut self,
        values_updated: bool,
        editor: &mut Editor,
        completer: &mut dyn Completer,
    ) -> bool {
        if !values_updated {
            self.update_values(editor, completer);
        }
        menu_functions::can_partially_complete(self.get_values(), editor)
    }

    fn menu_event(&mut self, event: MenuEvent) {
        match &event {
            MenuEvent::Activate(_) => {
                self.active = true;
                self.loaded = false;
            }
            MenuEvent::Deactivate => {
                self.active = false;
                self.loaded = false;
                self.state.stop();
            }
            _ => {}
        }
        self.event = Some(event);
    }

    fn update_values(&mut self, editor: &mut Editor, completer: &mut dyn Completer) {
        let (input, pos) = menu_functions::completer_input(
            editor.get_buffer(),
            editor.line_buffer().insertion_point(),
            None,
            false,
        );
        let (values, _) = completer.complete_with_base_ranges(&input, pos);
        self.values = values;
        self.loaded = false;
    }

    fn update_working_details(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        painter: &Painter,
    ) {
        if let Some(event) = self.event.take() {
            match event {
                MenuEvent::Activate(updated) => {
                    if !updated {
                        self.update_values(editor, completer);
                    }
                    self.load_suggestions(painter.screen_width());
                }
                MenuEvent::Deactivate => {}
                MenuEvent::Edit(updated) => {
                    if !updated {
                        self.update_values(editor, completer);
                    }
                    self.loaded = false;
                    self.load_suggestions(painter.screen_width());
                }
                MenuEvent::NextElement => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self.state.process_action(MenuAction::Next);
                }
                MenuEvent::PreviousElement => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self.state.process_action(MenuAction::Prev);
                }
                MenuEvent::MoveUp => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self.state.process_action(MenuAction::Up);
                }
                MenuEvent::MoveDown => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self.state.process_action(MenuAction::Down);
                }
                MenuEvent::MoveLeft => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self.state.process_action(MenuAction::Left);
                }
                MenuEvent::MoveRight => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self.state.process_action(MenuAction::Right);
                }
                MenuEvent::NextPage => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self.state.process_action(MenuAction::PageDown);
                }
                MenuEvent::PreviousPage => {
                    self.load_suggestions(painter.screen_width());
                    let _ = self.state.process_action(MenuAction::PageUp);
                }
            }
        }
    }

    fn replace_in_buffer(&self, editor: &mut Editor) {
        let value = self.get_values().get(self.index()).cloned();
        menu_functions::replace_in_buffer(value, editor);
    }

    fn min_rows(&self) -> u16 {
        3
    }

    fn get_values(&self) -> &[Suggestion] {
        &self.values
    }

    fn menu_required_lines(&self, _terminal_columns: u16) -> u16 {
        // Estimate from item count and columns
        let cols = self.state.cols().max(1);
        let rows = self.values.len().div_ceil(cols);
        (rows as u16).max(3)
    }

    fn menu_string(&self, available_lines: u16, _use_ansi_coloring: bool) -> String {
        // Use a mutable clone for rendering (Menu trait takes &self)
        let mut state = self.state.clone();
        state.set_available_rows(available_lines as usize);
        let rendering = state.render();

        let mut output = String::new();
        for (i, line) in rendering.lines.iter().enumerate() {
            if i > 0 {
                output.push_str("\r\n");
            }
            output.push_str(&line.content);
        }
        if rendering.lines.is_empty() {
            output.push_str("NO RECORDS FOUND");
        }
        output
    }
}

// ============================================================================
// COMPLETER
// ============================================================================

struct ZshrsCompleter {
    cache: CompsysCache,
    /// Path to the SQLite cache file — needed by completion threads that open
    /// their own read-only connections to avoid Send issues with rusqlite.
    cache_path: PathBuf,
    #[allow(dead_code)]
    comp_state: CompletionState,
}

impl ZshrsCompleter {
    fn new(mut cache: CompsysCache, cache_path: PathBuf) -> Self {
        // Check if completion mappings need to be built
        let (valid, count) = compinit_lazy(&cache);
        if !valid || count == 0 {
            // Build cache from fpath
            let fpath = get_system_fpath();
            let _ = build_cache_from_fpath(&fpath, &mut cache);
        }

        Self {
            cache,
            cache_path,
            comp_state: CompletionState::new(),
        }
    }
}

impl Completer for ZshrsCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let line_to_pos = &line[..pos];
        let word_start = line_to_pos
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let current_word = &line_to_pos[word_start..];
        let current_word = current_word.trim_start_matches('@');

        let words: Vec<&str> = line_to_pos.split_whitespace().collect();
        let is_first_word = words.len() <= 1 && !line_to_pos.ends_with(' ');

        if is_first_word {
            // Command position — launch executable, builtin, and function lookups
            // on separate threads to overlap SQLite I/O and string matching.
            if current_word.is_empty() {
                return Vec::new();
            }

            let prefix = current_word.to_string();
            let prefix_lower = current_word.to_lowercase();
            let ws = word_start;
            let p = pos;

            // --- Thread 1: executables from SQLite FTS cache ---
            let cache_path = self.cache_path.clone();
            let prefix_exec = prefix.clone();
            let exec_handle = std::thread::spawn(move || -> Vec<Suggestion> {
                let mut results = Vec::new();
                if let Ok(cache) = zsh::compsys::cache::CompsysCache::open(&cache_path) {
                    if let Ok(executables) = cache.get_executables_prefix_fts(&prefix_exec) {
                        for (name, path) in executables.into_iter().take(100) {
                            results.push(Suggestion {
                                value: name,
                                description: Some(path),
                                style: None,
                                extra: Some(vec!["command".to_string()]),
                                span: Span::new(ws, p),
                                append_whitespace: true,
                                display_override: None,
                                match_indices: None,
                            });
                        }
                    }
                }
                results
            });

            // --- Thread 2: builtin matching (pure CPU, fast) ---
            let prefix_builtin = prefix_lower.clone();
            let builtin_handle = std::thread::spawn(move || -> Vec<Suggestion> {
                let builtins = [
                    "alias",
                    "autoload",
                    "bg",
                    "bindkey",
                    "break",
                    "builtin",
                    "cd",
                    "command",
                    "compctl",
                    "continue",
                    "declare",
                    "dirs",
                    "disown",
                    "echo",
                    "emulate",
                    "enable",
                    "eval",
                    "exec",
                    "exit",
                    "export",
                    "false",
                    "fc",
                    "fg",
                    "float",
                    "functions",
                    "getopts",
                    "hash",
                    "history",
                    "integer",
                    "jobs",
                    "kill",
                    "let",
                    "limit",
                    "local",
                    "log",
                    "logout",
                    "noglob",
                    "popd",
                    "print",
                    "printf",
                    "pushd",
                    "pwd",
                    "read",
                    "readonly",
                    "rehash",
                    "return",
                    "set",
                    "setopt",
                    "shift",
                    "source",
                    "suspend",
                    "test",
                    "times",
                    "trap",
                    "true",
                    "type",
                    "typeset",
                    "ulimit",
                    "umask",
                    "unalias",
                    "unfunction",
                    "unhash",
                    "unlimit",
                    "unset",
                    "unsetopt",
                    "wait",
                    "whence",
                    "where",
                    "which",
                    "zcompile",
                    "zformat",
                    "zle",
                    "zmodload",
                    "zparseopts",
                    "zprof",
                    "zpty",
                    "zregexparse",
                    "zsocket",
                    "zstat",
                    "zstyle",
                ];
                let mut results = Vec::new();
                for builtin in builtins {
                    if builtin.starts_with(&prefix_builtin) {
                        results.push(Suggestion {
                            value: builtin.to_string(),
                            description: Some("builtin".to_string()),
                            style: None,
                            extra: Some(vec!["builtin".to_string()]),
                            span: Span::new(ws, p),
                            append_whitespace: true,
                            display_override: None,
                            match_indices: None,
                        });
                    }
                }
                results
            });

            // --- Thread 3: shell functions from SQLite cache ---
            let cache_path2 = self.cache_path.clone();
            let prefix_func = prefix.clone();
            let func_handle = std::thread::spawn(move || -> Vec<Suggestion> {
                let mut results = Vec::new();
                if let Ok(cache) = zsh::compsys::cache::CompsysCache::open(&cache_path2) {
                    if let Ok(funcs) = cache.get_shell_functions_prefix(&prefix_func) {
                        for (name, source) in funcs.into_iter().take(50) {
                            results.push(Suggestion {
                                value: name,
                                description: Some(source),
                                style: None,
                                extra: Some(vec!["function".to_string()]),
                                span: Span::new(ws, p),
                                append_whitespace: true,
                                display_override: None,
                                match_indices: None,
                            });
                        }
                    }
                }
                results
            });

            // Collect results from all threads
            let mut suggestions = Vec::new();
            if let Ok(mut exec_results) = exec_handle.join() {
                suggestions.append(&mut exec_results);
            }
            if let Ok(mut builtin_results) = builtin_handle.join() {
                suggestions.append(&mut builtin_results);
            }
            if let Ok(mut func_results) = func_handle.join() {
                suggestions.append(&mut func_results);
            }

            tracing::trace!(
                count = suggestions.len(),
                prefix = %prefix,
                "parallel command completion complete"
            );

            suggestions.sort_by(|a, b| a.value.cmp(&b.value));
            suggestions.dedup_by(|a, b| a.value == b.value);
            return suggestions;
        }

        let mut suggestions = Vec::new();

        if current_word.starts_with('-') {
            // Option completion - use compsys cache to find options
            if let Some(cmd) = words.first() {
                // Try to get options from completion function in cache
                if let Ok(Some(func)) = self.cache.get_comp(cmd) {
                    if let Ok(Some(stub)) = self.cache.get_autoload(&func) {
                        if let Ok(content) = std::fs::read_to_string(&stub.source) {
                            let prefix_lower = current_word.to_lowercase();
                            for line in content.lines() {
                                let line = line.trim();
                                if !line.contains('[') || line.starts_with('#') {
                                    continue;
                                }
                                for segment in line.split('\'') {
                                    if let Some((opt, desc)) = parse_option_spec(segment) {
                                        if opt.to_lowercase().starts_with(&prefix_lower) {
                                            suggestions.push(Suggestion {
                                                value: opt,
                                                description: if desc.is_empty() {
                                                    None
                                                } else {
                                                    Some(desc)
                                                },
                                                style: None,
                                                extra: Some(vec!["option".to_string()]),
                                                span: Span::new(word_start, pos),
                                                append_whitespace: true,
                                                display_override: None,
                                                match_indices: None,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Fallback: hardcoded options for common commands
                if suggestions.is_empty() {
                    let options: Vec<(&str, &str)> = match *cmd {
                        "ls" => vec![
                            ("-l", "long listing"),
                            ("-a", "show hidden"),
                            ("-h", "human sizes"),
                            ("-R", "recursive"),
                            ("-t", "sort by time"),
                            ("-S", "sort by size"),
                            ("-r", "reverse"),
                            ("-1", "one per line"),
                            ("-d", "directories"),
                            ("-F", "indicators"),
                            ("--color", "colorize"),
                            ("--help", "help"),
                        ],
                        "git" => vec![
                            ("--version", "version"),
                            ("--help", "help"),
                            ("-C", "path"),
                            ("-c", "config"),
                        ],
                        "grep" | "rg" => vec![
                            ("-i", "ignore case"),
                            ("-v", "invert"),
                            ("-r", "recursive"),
                            ("-n", "line numbers"),
                            ("-l", "files only"),
                            ("-c", "count"),
                        ],
                        "cargo" => vec![
                            ("--help", "help"),
                            ("--version", "version"),
                            ("-v", "verbose"),
                            ("-q", "quiet"),
                        ],
                        "cd" => vec![
                            ("-", "previous"),
                            ("-L", "follow symlinks"),
                            ("-P", "physical"),
                        ],
                        _ => vec![
                            ("--help", "help"),
                            ("--version", "version"),
                            ("-h", "help"),
                            ("-v", "verbose"),
                        ],
                    };
                    for (opt, desc) in options {
                        if opt.starts_with(current_word) {
                            suggestions.push(Suggestion {
                                value: opt.to_string(),
                                description: Some(desc.to_string()),
                                style: None,
                                extra: Some(vec!["option".to_string()]),
                                span: Span::new(word_start, pos),
                                append_whitespace: true,
                                display_override: None,
                                match_indices: None,
                            });
                        }
                    }
                }
            }
        } else {
            // Argument position - complete files
            let (dir, file_prefix) = if current_word.contains('/') {
                let idx = current_word.rfind('/').unwrap();
                let dir = if idx == 0 { "/" } else { &current_word[..idx] };
                (dir.to_string(), &current_word[idx + 1..])
            } else {
                (".".to_string(), current_word)
            };

            let dir_path = if dir.starts_with('~') {
                dirs::home_dir()
                    .map(|h| dir.replacen('~', &h.to_string_lossy(), 1))
                    .unwrap_or(dir.clone())
            } else {
                dir.clone()
            };

            if let Ok(entries) = std::fs::read_dir(&dir_path) {
                let prefix_lower = file_prefix.to_lowercase();
                for entry in entries.take(100).flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.to_lowercase().starts_with(&prefix_lower) || file_prefix.is_empty()
                        {
                            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            let display = if dir == "." {
                                name.to_string()
                            } else if dir.ends_with('/') {
                                format!("{}{}", dir, name)
                            } else {
                                format!("{}/{}", dir, name)
                            };
                            let value = if is_dir {
                                format!("{}/", display)
                            } else {
                                display
                            };
                            suggestions.push(Suggestion {
                                value,
                                description: if is_dir {
                                    Some("directory".to_string())
                                } else {
                                    None
                                },
                                style: None,
                                extra: Some(vec!["file".to_string()]),
                                span: Span::new(word_start, pos),
                                append_whitespace: !is_dir,
                                display_override: None,
                                match_indices: None,
                            });
                        }
                    }
                }
            }
        }

        // Deduplicate by value
        suggestions.sort_by(|a, b| a.value.cmp(&b.value));
        suggestions.dedup_by(|a, b| a.value == b.value);
        suggestions
    }
}

fn parse_option_spec(spec: &str) -> Option<(String, String)> {
    let spec = spec.trim();
    if !spec.contains('-') {
        return None;
    }
    let opt_start = if spec.starts_with('(') {
        spec.find(')')?.checked_add(1)?
    } else {
        0
    };
    let rest = &spec[opt_start..];
    if !rest.starts_with('-') {
        return None;
    }
    let opt_end = rest.find(['[', '=', ':', ' ']).unwrap_or(rest.len());
    let opt_name = rest[..opt_end].trim_end_matches(['+', '=']);
    if opt_name.is_empty() || opt_name == "-" || opt_name == "--" {
        return None;
    }
    let desc = if let Some(bracket_start) = rest.find('[') {
        if let Some(bracket_end) = rest[bracket_start..].find(']') {
            rest[bracket_start + 1..bracket_start + bracket_end].to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    Some((opt_name.to_string(), desc))
}

/// Custom prompt that supports PS1/PROMPT with zsh escape sequences
struct ZshrsPrompt {
    left_prompt: String,
    right_prompt: String,
}

impl ZshrsPrompt {
    fn new(executor: &ShellExecutor) -> Self {
        // Check for PS1 or PROMPT (zsh uses PROMPT, bash uses PS1)
        let prompt_str = executor
            .scalar("PROMPT")
            .or_else(|| executor.scalar("PS1"))
            .or_else(|| env::var("PROMPT").ok())
            .or_else(|| env::var("PS1").ok())
            .unwrap_or_else(|| "%n@%m %1~ %# ".to_string());

        // Check for RPROMPT (right prompt, zsh feature)
        let rprompt_str = executor
            .scalar("RPROMPT")
            .or_else(|| env::var("RPROMPT").ok())
            .unwrap_or_default();

        let left_prompt = expand_prompt_escapes(&prompt_str, executor);
        let right_prompt = expand_prompt_escapes(&rprompt_str, executor);

        Self {
            left_prompt,
            right_prompt,
        }
    }
}

impl Prompt for ZshrsPrompt {
    fn render_prompt_left(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(&self.left_prompt)
    }

    fn render_prompt_right(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(&self.right_prompt)
    }

    fn render_prompt_indicator(
        &self,
        _edit_mode: reedline::PromptEditMode,
    ) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("")
    }

    fn render_prompt_multiline_indicator(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("> ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> std::borrow::Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        std::borrow::Cow::Owned(format!(
            "({}reverse-i-search)`{}': ",
            prefix, history_search.term
        ))
    }

    fn get_prompt_color(&self) -> Color {
        Color::Green
    }

    fn get_indicator_color(&self) -> Color {
        Color::Cyan
    }

    fn get_prompt_right_color(&self) -> Color {
        Color::AnsiValue(5)
    }

    fn right_prompt_on_last_line(&self) -> bool {
        true
    }
}

fn expand_prompt_escapes(prompt: &str, executor: &ShellExecutor) -> String {
    let mut result = String::new();
    let mut chars = prompt.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('n') => {
                    // Username
                    result.push_str(&env::var("USER").unwrap_or_else(|_| "user".to_string()));
                }
                Some('m') => {
                    // Hostname (short)
                    result.push_str(
                        &hostname::get()
                            .map(|h| {
                                h.to_string_lossy()
                                    .split('.')
                                    .next()
                                    .unwrap_or("localhost")
                                    .to_string()
                            })
                            .unwrap_or_else(|_| "localhost".to_string()),
                    );
                }
                Some('M') => {
                    // Hostname (full)
                    result.push_str(
                        &hostname::get()
                            .map(|h| h.to_string_lossy().to_string())
                            .unwrap_or_else(|_| "localhost".to_string()),
                    );
                }
                Some('~') | Some('d') => {
                    // Current directory (~ for home)
                    let cwd = env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "?".to_string());
                    if let Some(home) = dirs::home_dir() {
                        let home_str = home.to_string_lossy();
                        if cwd.starts_with(home_str.as_ref()) {
                            result.push('~');
                            result.push_str(&cwd[home_str.len()..]);
                        } else {
                            result.push_str(&cwd);
                        }
                    } else {
                        result.push_str(&cwd);
                    }
                }
                Some('/') => {
                    // Current directory (full path)
                    result.push_str(
                        &env::current_dir()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| "?".to_string()),
                    );
                }
                Some('1') | Some('c') | Some('C') => {
                    // Trailing component of current directory
                    let cwd = env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "?".to_string());
                    if let Some(name) = PathBuf::from(&cwd).file_name() {
                        result.push_str(&name.to_string_lossy());
                    } else {
                        result.push('/');
                    }
                }
                Some('#') | Some('%') => {
                    // # if root, % otherwise
                    let is_root = env::var("EUID")
                        .or_else(|_| env::var("UID"))
                        .map(|uid| uid == "0")
                        .unwrap_or(false);
                    if is_root {
                        result.push('#');
                    } else {
                        result.push('%');
                    }
                }
                Some('?') => {
                    // Exit status of last command
                    result.push_str(&executor.last_status().to_string());
                }
                Some('j') => {
                    // Number of jobs
                    result.push_str(&executor.jobs.count().to_string());
                }
                Some('T') => {
                    // Current time in 12-hour format
                    let now = chrono::Local::now();
                    result.push_str(&now.format("%I:%M").to_string());
                }
                Some('t') | Some('@') => {
                    // Current time in 12-hour format with am/pm
                    let now = chrono::Local::now();
                    result.push_str(&now.format("%I:%M %p").to_string());
                }
                Some('*') => {
                    // Current time in 24-hour format
                    let now = chrono::Local::now();
                    result.push_str(&now.format("%H:%M").to_string());
                }
                Some('D') => {
                    // Date
                    let now = chrono::Local::now();
                    result.push_str(&now.format("%Y-%m-%d").to_string());
                }
                Some('F') => {
                    // Bold (start)
                    result.push_str("\x1b[1m");
                }
                Some('f') => {
                    // Bold (end) / reset
                    result.push_str("\x1b[0m");
                }
                Some('B') => {
                    // Bold (start, alternative)
                    result.push_str("\x1b[1m");
                }
                Some('b') => {
                    // Bold (end, alternative)
                    result.push_str("\x1b[22m");
                }
                Some('{') => {
                    // Start of literal escape sequence (ignored)
                }
                Some('}') => {
                    // End of literal escape sequence (ignored)
                }
                Some(other) => {
                    result.push('%');
                    result.push(other);
                }
                None => {
                    result.push('%');
                }
            }
        } else if c == '\\' {
            // Bash-style escapes
            match chars.next() {
                Some('u') => {
                    result.push_str(&env::var("USER").unwrap_or_else(|_| "user".to_string()));
                }
                Some('h') => {
                    result.push_str(
                        &hostname::get()
                            .map(|h| {
                                h.to_string_lossy()
                                    .split('.')
                                    .next()
                                    .unwrap_or("localhost")
                                    .to_string()
                            })
                            .unwrap_or_else(|_| "localhost".to_string()),
                    );
                }
                Some('H') => {
                    result.push_str(
                        &hostname::get()
                            .map(|h| h.to_string_lossy().to_string())
                            .unwrap_or_else(|_| "localhost".to_string()),
                    );
                }
                Some('w') => {
                    let cwd = env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "?".to_string());
                    if let Some(home) = dirs::home_dir() {
                        let home_str = home.to_string_lossy();
                        if cwd.starts_with(home_str.as_ref()) {
                            result.push('~');
                            result.push_str(&cwd[home_str.len()..]);
                        } else {
                            result.push_str(&cwd);
                        }
                    } else {
                        result.push_str(&cwd);
                    }
                }
                Some('W') => {
                    let cwd = env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "?".to_string());
                    if let Some(name) = PathBuf::from(&cwd).file_name() {
                        result.push_str(&name.to_string_lossy());
                    } else {
                        result.push('/');
                    }
                }
                Some('$') => {
                    let is_root = env::var("EUID")
                        .or_else(|_| env::var("UID"))
                        .map(|uid| uid == "0")
                        .unwrap_or(false);
                    if is_root {
                        result.push('#');
                    } else {
                        result.push('$');
                    }
                }
                Some('n') => {
                    result.push('\n');
                }
                Some('r') => {
                    result.push('\r');
                }
                Some('\\') => {
                    result.push('\\');
                }
                Some('[') | Some(']') => {
                    // Non-printing character markers (ignored in output)
                }
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => {
                    result.push('\\');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}
