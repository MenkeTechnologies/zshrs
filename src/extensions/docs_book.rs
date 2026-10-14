//! `zshrs --docs` — the reference book.
//!
//! zshrs-original. With a topic, `zshrs --docs NAME` prints the hover card
//! the LSP returns for NAME (`lsp::lookup_doc`). Without one it opens the
//! whole reference as a paged book in the terminal — the same interface as
//! `stryke docs`: an introduction, chapters, a table of contents, search,
//! and vim-style keys. The pages are not a second copy of the docs: every
//! one is `lookup_doc`'s card for a name taken from the tables the LSP
//! already serves (reserved words, operators, zsh builtins, zshrs builtins,
//! options, special parameters), rendered by `render_doc_card`.
//!
//! The interactive book runs only when stdin and stdout are both terminals
//! and `ZSHRS_NO_TTY` is unset; anything else prints and exits, so a pipe,
//! CI job or editor can never block on a key read.

use std::collections::HashSet;
use std::io::{IsTerminal, Write};

/// One documented topic: the chapter it sits in, its name, and the raw
/// markdown card `lsp::lookup_doc` returns for it.
pub struct Entry {
    pub chapter: &'static str,
    pub name: String,
    pub card: String,
}

/// Every topic, in book order. A chapter lists the names of one doc table;
/// a name whose card is empty, or identical to a card already placed (an
/// alias such as `PATH` for `$path`), is skipped.
pub fn entries() -> Vec<Entry> {
    let mut chapters: Vec<(&'static str, Vec<String>)> = Vec::new();

    // Only names whose card IS a keyword card: RESWDS also holds the typeset
    // family (`declare`, `export`, …), documented as builtins and placed in
    // that chapter.
    let mut words: Vec<String> = crate::ported::hashtable::RESWDS
        .iter()
        .map(|(n, _)| n.to_string())
        .chain(crate::zsh_keyword_docs::KEYWORD_DOCS.iter().map(|(n, _)| topic_name(n).to_string()))
        .filter(|n| crate::lsp::lookup_doc(doc_key(n)).contains("_zsh keyword_"))
        .collect();
    words.sort();
    words.dedup();
    chapters.push(("Reserved words", words));

    chapters.push((
        "Operators",
        crate::lsp::OPERATOR_DOCS.iter().map(|(n, _)| n.to_string()).collect(),
    ));

    let extension: HashSet<&str> = crate::ext_builtins::EXT_BUILTIN_NAMES
        .iter()
        .chain(crate::daemon::builtins::ZSHRS_BUILTIN_NAMES.iter())
        .copied()
        .collect();
    let mut zsh_builtins: Vec<String> = crate::ported::builtin::BUILTINS
        .iter()
        .map(|b| b.node.nam.to_string())
        .filter(|n| !extension.contains(n.as_str()))
        .collect();
    zsh_builtins.sort();
    chapters.push(("Builtins", zsh_builtins));

    let mut zshrs_builtins: Vec<String> = extension.iter().map(|n| n.to_string()).collect();
    zshrs_builtins.sort();
    chapters.push(("zshrs builtins", zshrs_builtins));

    chapters.push((
        "Options",
        crate::zsh_option_docs::OPTION_DOCS.iter().map(|(n, _)| n.to_string()).collect(),
    ));
    chapters.push((
        "Special parameters",
        crate::zsh_special_var_docs::SPECIAL_VAR_DOCS
            .iter()
            .map(|(n, _)| n.to_string())
            .collect(),
    ));

    let mut out = Vec::new();
    let mut names: HashSet<String> = HashSet::new();
    let mut cards: HashSet<String> = HashSet::new();
    for (chapter, list) in chapters {
        for name in list {
            if names.contains(&name) {
                continue;
            }
            let card = crate::lsp::lookup_doc(doc_key(&name));
            if card.is_empty() || cards.contains(&card) {
                continue;
            }
            names.insert(name.clone());
            cards.insert(card.clone());
            out.push(Entry { chapter, name, card });
        }
    }
    out
}

/// zsh's yodl source files the `( list )` subshell under the token name
/// `LPAR`; the book shows it, and accepts it, as `(`.
fn topic_name(doc_name: &str) -> &str {
    if doc_name == "LPAR" { "(" } else { doc_name }
}

/// The `lsp::lookup_doc` key for a topic name (inverse of `topic_name`).
fn doc_key(topic: &str) -> &str {
    if topic == "(" { "LPAR" } else { topic }
}

/// A screenful of the book: its chapter, the lines shown, and the entries
/// that start on it.
struct Page {
    chapter: String,
    lines: Vec<String>,
    starts: Vec<usize>,
}

/// The chapters in book order with their topic counts.
fn chapter_counts(entries: &[Entry]) -> Vec<(&'static str, usize)> {
    let mut out: Vec<(&'static str, usize)> = Vec::new();
    for e in entries {
        match out.last_mut() {
            Some((c, n)) if *c == e.chapter => *n += 1,
            _ => out.push((e.chapter, 1)),
        }
    }
    out
}

/// Lay the rendered cards out on pages of `height` lines. A chapter always
/// starts a page; a card taller than a page continues on the next ones.
fn build_pages(entries: &[Entry], height: usize, color: bool) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    let mut cur = Page { chapter: String::new(), lines: Vec::new(), starts: Vec::new() };
    for (i, e) in entries.iter().enumerate() {
        let card: Vec<String> = render_doc_card(&e.name, &e.card, color)
            .lines()
            .map(str::to_string)
            .collect();
        let new_chapter = cur.chapter != e.chapter;
        let fits = cur.lines.len() + card.len() + 1 <= height;
        if !cur.lines.is_empty() && (new_chapter || !fits) {
            pages.push(std::mem::replace(
                &mut cur,
                Page { chapter: String::new(), lines: Vec::new(), starts: Vec::new() },
            ));
        }
        cur.chapter = e.chapter.to_string();
        cur.starts.push(i);
        if !cur.lines.is_empty() {
            cur.lines.push(String::new());
        }
        for line in card {
            if cur.lines.len() == height {
                let chapter = cur.chapter.clone();
                pages.push(std::mem::replace(
                    &mut cur,
                    Page { chapter, lines: Vec::new(), starts: Vec::new() },
                ));
            }
            cur.lines.push(line);
        }
    }
    if !cur.lines.is_empty() {
        pages.push(cur);
    }
    pages
}

const KEYS: &[(&str, &str)] = &[
    ("j  n  space  enter", "next page"),
    ("k  p", "previous page"),
    ("]  [", "next / previous chapter"),
    ("d  u", "forward / back 5 pages"),
    ("g  G", "first / last page"),
    ("t", "table of contents"),
    ("/", "search every topic"),
    (":", "jump to a page number"),
    ("r", "random page"),
    ("?", "this key list"),
    ("q  esc", "quit"),
];

struct Theme {
    c: &'static str,
    g: &'static str,
    d: &'static str,
    b: &'static str,
    n: &'static str,
}

impl Theme {
    fn new(color: bool) -> Self {
        if color {
            Theme { c: "\x1b[36m", g: "\x1b[32m", d: "\x1b[2m", b: "\x1b[1m", n: "\x1b[0m" }
        } else {
            Theme { c: "", g: "", d: "", b: "", n: "" }
        }
    }
}

/// The opening page: what the book holds and how to move around it.
fn intro_lines(entries: &[Entry], t: &Theme) -> Vec<String> {
    let Theme { c, g, d, b, n } = t;
    let chapters = chapter_counts(entries);
    let mut out = vec![
        format!("  {d}>> THE ZSHRS REFERENCE // INTERACTIVE DOCUMENTATION <<{n}"),
        String::new(),
        format!(
            "  {b}Every reserved word, operator, builtin, option and special{n}"
        ),
        format!(
            "  {b}parameter. {g}{}{n} {b}topics in {g}{}{n} {b}chapters.{n}",
            entries.len(),
            chapters.len()
        ),
        String::new(),
        format!("  {d}── KEYS ──────────────────────────────────────────────{n}"),
    ];
    for (k, what) in KEYS {
        out.push(format!("  {c}{k:<20}{n} {what}"));
    }
    out.push(String::new());
    out.push(format!("  {d}── CHAPTERS ──────────────────────────────────────────{n}"));
    for (i, (chapter, count)) in chapters.iter().enumerate() {
        out.push(format!("  {c}{:>2}.{n} {b}{chapter:<24}{n} {d}{count} topics{n}", i + 1));
    }
    out
}

/// `--toc`: each chapter with the page it starts on, then its topics.
fn toc_lines(entries: &[Entry], pages: &[Page], t: &Theme, with_topics: bool) -> Vec<String> {
    let Theme { c, d, b, n, .. } = t;
    let mut out = Vec::new();
    for (chapter, count) in chapter_counts(entries) {
        let first = pages.iter().position(|p| p.chapter == chapter).unwrap_or(0);
        out.push(format!(
            "  {b}{chapter:<24}{n} {d}{count} topics, page {}{n}",
            first + 2
        ));
        if with_topics {
            for (pi, p) in pages.iter().enumerate().filter(|(_, p)| p.chapter == chapter) {
                for &ei in &p.starts {
                    out.push(format!("      {c}{:<28}{n} {d}{}{n}", entries[ei].name, pi + 2));
                }
            }
        }
    }
    out
}

/// Pages whose topic name or card contains `pat` (case-insensitive), as
/// (page number, topic name) — page 1 is the introduction.
fn search(entries: &[Entry], pages: &[Page], pat: &str) -> Vec<(usize, String)> {
    let pat = pat.to_lowercase();
    let mut out = Vec::new();
    for (pi, p) in pages.iter().enumerate() {
        for &ei in &p.starts {
            let e = &entries[ei];
            if e.name.to_lowercase().contains(&pat) || e.card.to_lowercase().contains(&pat) {
                out.push((pi + 2, e.name.clone()));
            }
        }
    }
    out
}

fn print_help(t: &Theme) {
    let Theme { c, d, b, n, .. } = t;
    println!("  {b}USAGE:{n} zshrs --docs {d}[OPTION | PAGE | TOPIC]{n}");
    println!();
    println!("  {c}(none){n}              open the book (interactive on a terminal)");
    println!("  {c}TOPIC{n}               print TOPIC's entry (`zshrs --docs typeset`)");
    println!("  {c}PAGE{n}                print page PAGE");
    println!("  {c}-t, --toc{n}           table of contents with every topic's page");
    println!("  {c}-l, --list{n}          every topic, one per line");
    println!("  {c}-s, --search PAT{n}    topics whose name or text contains PAT");
    println!("  {c}-h, --help{n}          this message");
    println!();
    println!("  {d}Keys in the book:{n}");
    for (k, what) in KEYS {
        println!("  {c}{k:<20}{n} {what}");
    }
}

/// `zshrs --docs [ARGS]`. `args` are the words after `--docs`; `color` is
/// the caller's `--color` / NO_COLOR / FORCE_COLOR / TTY decision.
pub fn run(args: &[String], color: bool) -> i32 {
    let t = Theme::new(color);
    let first = args.first().map(String::as_str);
    if matches!(first, Some("-h" | "--help")) {
        print_help(&t);
        return 0;
    }
    // The options; any other word is a topic, `-` and `-eq` included.
    const OPTIONS: &[&str] = &["-t", "--toc", "-l", "--list", "-s", "--search"];
    if let Some(a) = first.filter(|a| !OPTIONS.contains(a) && a.parse::<usize>().is_err()) {
        if a.starts_with('-') && crate::lsp::lookup_doc(a).is_empty() {
            eprintln!("zshrs: --docs: bad option: {a}");
            return 1;
        }
        return print_topic(a, color);
    }

    let entries = entries();
    let (rows, _) = term_size();
    let height = rows.saturating_sub(4).max(8);
    let pages = build_pages(&entries, height, color);
    let total = pages.len() + 1; // + the introduction

    match first {
        Some("-l" | "--list") => {
            for e in &entries {
                println!("{}", e.name);
            }
            return 0;
        }
        Some("-t" | "--toc") => {
            for l in toc_lines(&entries, &pages, &t, true) {
                println!("{l}");
            }
            return 0;
        }
        Some("-s" | "--search") => {
            let Some(pat) = args.get(1) else {
                eprintln!("zshrs: --docs --search: pattern expected");
                return 1;
            };
            let hits = search(&entries, &pages, pat);
            for (page, name) in &hits {
                println!("  {}{page:>5}{}  {name}", t.c, t.n);
            }
            return if hits.is_empty() { 1 } else { 0 };
        }
        _ => {}
    }

    let start = match first.map(str::parse::<usize>) {
        Some(Ok(n)) if (1..=total).contains(&n) => n - 1,
        Some(_) => {
            eprintln!("zshrs: --docs: no page {} (1-{total})", first.unwrap_or(""));
            return 1;
        }
        None => 0,
    };
    let intro = intro_lines(&entries, &t);
    let interactive = first.is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::env::var_os("ZSHRS_NO_TTY").is_none();
    if !interactive {
        let lines = if start == 0 { &intro } else { &pages[start - 1].lines };
        for l in lines {
            println!("{l}");
        }
        return 0;
    }
    match browse(&entries, &pages, &intro, start, height, &t) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("zshrs: --docs: {e}");
            1
        }
    }
}

/// `zshrs --docs NAME`: the hover card for NAME — styled when `color`,
/// otherwise the raw markdown, so `zshrs --docs X | glow` and the editor
/// popup that renders markdown itself get the `**name**` heading intact.
fn print_topic(name: &str, color: bool) -> i32 {
    let card = crate::lsp::lookup_doc(doc_key(name));
    if card.is_empty() {
        eprintln!("zshrs: no docs for {name}");
        return 1;
    }
    if color {
        print!("{}", render_doc_card(name, &card, true));
    } else {
        print!("{card}");
        if !card.ends_with('\n') {
            println!();
        }
    }
    0
}

/// Terminal rows and columns; 24x80 when there is no terminal.
fn term_size() -> (usize, usize) {
    match crossterm::terminal::size() {
        Ok((cols, rows)) if rows > 0 && cols > 0 => (rows as usize, cols as usize),
        _ => (24, 80),
    }
}

/// Raw mode and the alternate screen for the life of the book; dropped on
/// every exit path, including an error or a panic unwinding through it.
struct Screen;

impl Screen {
    fn enter() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        let mut out = std::io::stdout();
        crossterm::execute!(out, crossterm::terminal::EnterAlternateScreen, crossterm::cursor::Hide)?;
        Ok(Screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let mut out = std::io::stdout();
        let _ = crossterm::execute!(out, crossterm::cursor::Show, crossterm::terminal::LeaveAlternateScreen);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// What the screen shows besides a numbered page.
enum View {
    Page(usize),
    Toc,
    Keys,
    Results(String, Vec<(usize, String)>),
}

fn draw(out: &mut impl Write, header: &str, body: &[String], status: &str, height: usize, t: &Theme) -> std::io::Result<()> {
    let Theme { c, d, n, .. } = t;
    write!(out, "\x1b[H\x1b[2J")?;
    write!(out, "{c}{header}{n}\r\n{d}{}{n}\r\n", "─".repeat(visible_width(header).max(20)))?;
    for line in body.iter().take(height) {
        write!(out, "{line}\r\n")?;
    }
    for _ in body.len().min(height)..height {
        write!(out, "\r\n")?;
    }
    write!(out, "{d}{status}{n}")?;
    out.flush()
}

/// Read a line at the bottom row after `prompt`; None on Esc.
fn read_line(out: &mut impl Write, prompt: &str, row: u16) -> std::io::Result<Option<String>> {
    use crossterm::event::{read, Event, KeyCode, KeyEventKind};
    let mut s = String::new();
    loop {
        write!(out, "\x1b[{};1H\x1b[2K{prompt}{s}", row + 1)?;
        out.flush()?;
        if let Event::Key(k) = read()? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match k.code {
                KeyCode::Enter => return Ok(Some(s)),
                KeyCode::Esc => return Ok(None),
                KeyCode::Backspace => {
                    s.pop();
                }
                KeyCode::Char(ch) => s.push(ch),
                _ => {}
            }
        }
    }
}

fn browse(entries: &[Entry], pages: &[Page], intro: &[String], start: usize, height: usize, t: &Theme) -> std::io::Result<()> {
    use crossterm::event::{read, Event, KeyCode, KeyEventKind, KeyModifiers};
    let _screen = Screen::enter()?;
    let mut out = std::io::stdout();
    let total = pages.len() + 1;
    let mut cur = start;
    let mut view = View::Page(cur);
    let toc = toc_lines(entries, pages, t, false);
    let keys: Vec<String> = KEYS.iter().map(|(k, w)| format!("  {}{k:<20}{} {w}", t.c, t.n)).collect();
    // A chapter's first page; page 0 is the introduction.
    let chapter_of = |p: usize| if p == 0 { "" } else { pages[p - 1].chapter.as_str() };
    loop {
        let (header, body): (String, Vec<String>) = match &view {
            View::Page(0) => ("ZSHRS DOCS // introduction".into(), intro.to_vec()),
            View::Page(p) => (
                format!("ZSHRS DOCS // {} // page {}/{total}", pages[p - 1].chapter, p + 1),
                pages[p - 1].lines.clone(),
            ),
            View::Toc => ("ZSHRS DOCS // table of contents".into(), toc.clone()),
            View::Keys => ("ZSHRS DOCS // keys".into(), keys.clone()),
            View::Results(pat, hits) => (
                format!("ZSHRS DOCS // search: {pat} // {} topics", hits.len()),
                hits.iter().map(|(p, name)| format!("  {}{p:>5}{}  {name}", t.c, t.n)).collect(),
            ),
        };
        draw(&mut out, &header, &body, "q quit  ? keys  t contents  / search  :N page", height, t)?;
        let Event::Key(k) = read()? else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        let page = |p: usize| View::Page(p.min(total - 1));
        view = match k.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
            KeyCode::Char('j' | 'n' | ' ' | 'l') | KeyCode::Enter | KeyCode::Down | KeyCode::PageDown | KeyCode::Right => {
                cur = (cur + 1).min(total - 1);
                page(cur)
            }
            KeyCode::Char('k' | 'p' | 'h') | KeyCode::Up | KeyCode::PageUp | KeyCode::Left => {
                cur = cur.saturating_sub(1);
                page(cur)
            }
            KeyCode::Char('d') => {
                cur = (cur + 5).min(total - 1);
                page(cur)
            }
            KeyCode::Char('u') => {
                cur = cur.saturating_sub(5);
                page(cur)
            }
            KeyCode::Char('g' | '0') | KeyCode::Home => {
                cur = 0;
                page(cur)
            }
            KeyCode::Char('G' | '$') | KeyCode::End => {
                cur = total - 1;
                page(cur)
            }
            KeyCode::Char(']' | '}') => {
                let here = chapter_of(cur);
                cur = (cur + 1..total).find(|&p| chapter_of(p) != here).unwrap_or(cur);
                page(cur)
            }
            KeyCode::Char('[' | '{') => {
                // Back to the start of this chapter, or of the previous one
                // when already there.
                let mut p = cur;
                if p > 0 && chapter_of(p - 1) != chapter_of(p) {
                    p -= 1;
                }
                let here = chapter_of(p);
                while p > 0 && chapter_of(p - 1) == here {
                    p -= 1;
                }
                cur = p;
                page(cur)
            }
            KeyCode::Char('r') => {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() as usize)
                    .unwrap_or(0);
                cur = 1 + nanos % (total - 1).max(1);
                page(cur)
            }
            KeyCode::Char('t') => View::Toc,
            KeyCode::Char('?') => View::Keys,
            KeyCode::Char('/') => match read_line(&mut out, "/", height as u16 + 2)? {
                Some(pat) if !pat.is_empty() => {
                    let hits = search(entries, pages, &pat);
                    if let [(p, _)] = hits.as_slice() {
                        cur = p - 1;
                        page(cur)
                    } else {
                        View::Results(pat, hits)
                    }
                }
                _ => page(cur),
            },
            KeyCode::Char(':') => match read_line(&mut out, ":", height as u16 + 2)? {
                Some(s) => match s.trim().parse::<usize>() {
                    Ok(n) if (1..=total).contains(&n) => {
                        cur = n - 1;
                        page(cur)
                    }
                    _ => page(cur),
                },
                None => page(cur),
            },
            _ => match view {
                View::Page(_) => page(cur),
                other => other,
            },
        };
    }
}

// ─── hover-card rendering (moved from bins/zshrs.rs) ─────────────────────

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
/// Inline backticks → cyan; `**bold**` → ANSI bold. With `color=false`
/// the escapes are empty strings, so the markers are consumed and the
/// result is plain unstyled text — NOT markdown (the `**` / `_` markers
/// are gone and the heading has been rewritten into a banner).
///
/// Used by `--docs NAME` on the coloured path and by every page of the book.
/// `--color never` / `NO_COLOR` / a non-TTY stdout without
/// FORCE_COLOR bypasses this function entirely and prints the raw
/// `lsp::lookup_doc` markdown, which is what a markdown-rendering
/// consumer needs.
pub fn render_doc_card(name: &str, card: &str, color: bool) -> String {
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

/// Terminal width; 80 columns when there is no terminal. An ioctl, not a
/// `stty size` subprocess: the book renders every card, thousands of them.
fn term_width() -> usize {
    term_size().1
}
