//! `zshrs --docs` — the reference book.
//!
//! zshrs-original. With a topic, `zshrs --docs NAME` prints the hover card
//! the LSP returns for NAME (`lsp::lookup_doc`), or, for a name only the
//! manual documents (a ZLE widget, a completion style, an expansion flag),
//! that manual entry. Without one it opens the whole reference as a paged
//! book in the terminal — the interface `stryke docs` has: an introduction,
//! chapters, a table of contents, search and vim-style keys.
//!
//! The book is the manuals the binary already carries, not a second copy:
//! zshrs's own pages (`man/man1/zshrs.1`, `zshrsall.1`) and every page of
//! zsh's manual (`vendor/zsh/man1`), read from `bundled_docs` and laid out
//! page by page in `zshall.1`'s order. Each man page is a part, each
//! `.SH` / `.SS` section a chapter, and each `.TP` item a topic; a
//! section's opening prose is a topic of its own. Two chapters come from
//! the LSP tables because the manuals do not cover them: the zshrs
//! extension builtins and the operators.
//!
//! The interactive book runs only when stdin and stdout are both terminals
//! and `ZSHRS_NO_TTY` is unset; anything else prints and exits, so a pipe,
//! CI job or editor can never block on a key read.

use std::collections::HashSet;
use std::io::{IsTerminal, Write};

/// What a topic's text is: an LSP hover card (markdown) or the troff lines
/// of a man-page item, rendered when the pages are laid out.
pub enum Body {
    Card(String),
    /// A man-page item: the full terms when they say more than the name
    /// (`typeset [ {+|-}AHU… ]`, `vi-forward-word (unbound) (w)`), then its text.
    Troff { synopsis: Vec<String>, lines: Vec<String> },
}

/// One documented topic: the part (man page) and chapter (section) it sits
/// in, its name, and its text.
pub struct Entry {
    pub part: String,
    pub chapter: String,
    pub name: String,
    pub body: Body,
}

impl Entry {
    /// The topic rendered for the terminal: a heading, then the text.
    fn render(&self, color: bool) -> Vec<String> {
        match &self.body {
            Body::Card(card) => render_doc_card(&self.name, card, color).lines().map(str::to_string).collect(),
            Body::Troff { synopsis, lines } => {
                let t = Theme::new(color);
                let mut out = vec![
                    format!("  {}{}{}", t.c, self.name, t.n),
                    format!("  {}{}{}", t.d, "─".repeat(self.name.chars().count().clamp(20, 60)), t.n),
                    format!("  {}{}{}", t.d, self.chapter, t.n),
                    String::new(),
                ];
                for s in synopsis {
                    out.push(format!("  {}{s}{}", t.b, t.n));
                }
                if !synopsis.is_empty() {
                    out.push(String::new());
                }
                out.extend(render_troff(lines, term_width().saturating_sub(2).max(40), &t));
                out
            }
        }
    }

    /// Plain text for searching: the name, the chapter and the body.
    fn text(&self) -> String {
        let body = match &self.body {
            Body::Card(card) => card.clone(),
            Body::Troff { synopsis, lines } => {
                synopsis.join("\n") + "\n" + &render_troff(lines, 1000, &Theme::new(false)).join("\n")
            }
        };
        format!("{}\n{}\n{body}", self.name, self.chapter)
    }
}

/// The man pages of the book, in reading order: zshrs's own, `zsh.1`, then
/// every page `zshall.1` includes, in its `.so` order (zsh's own chapter
/// order). Pages the bundle lacks are skipped.
fn man_pages() -> Vec<(String, String)> {
    let mut by_name: std::collections::HashMap<String, String> = crate::bundled_docs::entries()
        .into_iter()
        .filter_map(|(name, body)| {
            let page = name.strip_prefix("man/man1/").or_else(|| name.strip_prefix("man1/"))?;
            Some((page.to_string(), String::from_utf8_lossy(&body).into_owned()))
        })
        .collect();
    let mut order: Vec<String> = ["zshrs.1", "zshrsall.1", "zsh.1"].iter().map(|s| s.to_string()).collect();
    if let Some(all) = by_name.get("zshall.1") {
        order.extend(all.lines().filter_map(|l| l.strip_prefix(".so man1/")).map(str::to_string));
    }
    order
        .into_iter()
        .filter_map(|p| by_name.remove(&p).map(|text| (p.trim_end_matches(".1").to_string(), text)))
        .collect()
}

/// Every topic, in book order.
pub fn entries() -> Vec<Entry> {
    let mut out = Vec::new();
    let pages = man_pages();
    let mut pages = pages.into_iter().peekable();
    // zshrs's own pages first, then its guides and the LSP-only chapters,
    // then zsh's manual.
    while let Some((page, _)) = pages.peek() {
        if !page.starts_with("zshrs") {
            break;
        }
        let (page, text) = pages.next().unwrap();
        out.extend(parse_man(&page, &text));
    }
    for (file, text) in GUIDES {
        out.extend(parse_markdown(file, text));
    }
    out.extend(lsp_chapters());
    for (page, text) in pages {
        out.extend(parse_man(&page, &text));
    }
    out
}

/// zshrs's user-facing guides, compiled in so the book always matches the
/// build. The repository's porting notes, bug log and plans are not guides
/// and stay out.
const GUIDES: &[(&str, &str)] = &[
    ("README", include_str!("../../README.md")),
    ("INVENTIONS", include_str!("../../docs/INVENTIONS.md")),
    ("DESIGN_GOALS", include_str!("../../docs/DESIGN_GOALS.md")),
    ("DAEMON", include_str!("../../docs/DAEMON.md")),
    ("DAEMON_AS_SERVICE", include_str!("../../docs/DAEMON_AS_SERVICE.md")),
    ("SHELL_IDS", include_str!("../../docs/SHELL_IDS.md")),
    ("RECORDER", include_str!("../../docs/RECORDER.md")),
    ("RECORDER_PROTOCOL", include_str!("../../docs/RECORDER_PROTOCOL.md")),
    ("PLUGINS", include_str!("../../docs/PLUGINS.md")),
    ("PORTING_ZSH_PLUGIN", include_str!("../../docs/PORTING_ZSH_PLUGIN.md")),
    ("ZNATIVE", include_str!("../../docs/ZNATIVE.md")),
    ("COMPLETION_DISPATCH", include_str!("../../docs/COMPLETION_DISPATCH.md")),
    ("IN_EDITOR_COMPSYS_COMPLETION", include_str!("../../docs/IN_EDITOR_COMPSYS_COMPLETION.md")),
    ("hot_path_performance", include_str!("../../docs/hot_path_performance.md")),
    ("RFC", include_str!("../../docs/RFC.md")),
];

/// Split a markdown guide into topics: the `#` title names the part, each
/// `##` section is a chapter, and each `###` subsection a topic; the prose
/// before a section's first `###` is a topic named after the section.
/// Headings inside code fences are code, not structure.
fn parse_markdown(file: &str, text: &str) -> Vec<Entry> {
    // The first `# ` heading outside a code fence (a `# comment` in a shell
    // example is not a title).
    let mut in_fence = false;
    let title = text
        .lines()
        .find_map(|l| {
            if l.trim_start().starts_with("```") {
                in_fence = !in_fence;
            }
            l.strip_prefix("# ").filter(|_| !in_fence)
        })
        .map(|t| format!("{file} — {}", t.trim()))
        .unwrap_or_else(|| file.to_string());
    let mut out = Vec::new();
    let mut chapter = String::new();
    let mut name = String::new();
    let mut body: Vec<&str> = Vec::new();
    let mut fence = false;
    let mut flush = |chapter: &str, name: &str, body: &mut Vec<&str>| {
        let text = body.join("\n");
        if !name.is_empty() && !text.trim().is_empty() {
            out.push(Entry {
                part: title.clone(),
                chapter: format!("{file}: {}", if chapter.is_empty() { "introduction" } else { chapter }),
                name: name.to_string(),
                body: Body::Card(format!("**{name}** — _{file}: {chapter}_\n\n{}", text.trim())),
            });
        }
        body.clear();
    };
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        let heading = (!fence).then(|| line.strip_prefix("## ").or_else(|| line.strip_prefix("### ")));
        match heading.flatten() {
            Some(h) => {
                flush(&chapter, &name, &mut body);
                if line.starts_with("## ") {
                    chapter = h.trim().to_string();
                }
                name = h.trim().to_string();
            }
            None if line.starts_with("# ") && !fence => {}
            None => body.push(line),
        }
    }
    flush(&chapter, &name, &mut body);
    out
}

/// The zshrs extension builtins and the operators — documented in the LSP
/// tables, not in any man page.
fn lsp_chapters() -> Vec<Entry> {
    let mut builtins: Vec<&str> = crate::ext_builtins::EXT_BUILTIN_NAMES
        .iter()
        .chain(crate::daemon::builtins::ZSHRS_BUILTIN_NAMES.iter())
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    builtins.sort();
    let operators: Vec<&str> = crate::lsp::OPERATOR_DOCS.iter().map(|(n, _)| *n).collect();
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (chapter, names) in [("zshrs builtins", builtins), ("Operators", operators)] {
        for name in names {
            let card = crate::lsp::lookup_doc(name);
            if card.is_empty() || !seen.insert(card.clone()) {
                continue;
            }
            out.push(Entry {
                part: "zshrs builtins and operators".into(),
                chapter: chapter.into(),
                name: name.to_string(),
                body: Body::Card(card),
            });
        }
    }
    out
}

/// Split a man page into topics. Sections NAME and SEE ALSO carry no
/// reference text and are skipped; the page's NAME line titles its part.
fn parse_man(page: &str, text: &str) -> Vec<Entry> {
    let lines: Vec<&str> = text.lines().collect();
    let part = lines
        .iter()
        .position(|l| l.trim_matches('"').ends_with("NAME") && l.starts_with(".SH"))
        .and_then(|i| lines.get(i + 1))
        .map(|l| troff_plain(l))
        .unwrap_or_else(|| page.to_string());

    let mut out: Vec<Entry> = Vec::new();
    let mut sh = String::new();
    let mut ss = String::new();
    let mut skip = false;
    let mut depth = 0usize;
    // The topic being collected: its names, its body, and whether it is a
    // `.TP` item (vs a section's opening prose).
    let mut names: Vec<String> = Vec::new();
    let mut synopsis: Vec<String> = Vec::new();
    let mut body: Vec<String> = Vec::new();
    let mut is_item = false;
    let mut want_term = false;

    let chapter = |sh: &str, ss: &str| {
        if ss.is_empty() { format!("{page}: {sh}") } else { format!("{page}: {sh} › {ss}") }
    };
    let mut flush = |names: &mut Vec<String>, synopsis: &mut Vec<String>, body: &mut Vec<String>, sh: &str, ss: &str, skip: bool| {
        let has_text = body.iter().any(|l| !l.starts_with('.') && !l.trim().is_empty());
        if !skip && !names.is_empty() && has_text {
            out.push(Entry {
                part: part.clone(),
                chapter: chapter(sh, ss),
                name: names.join(", "),
                body: Body::Troff { synopsis: std::mem::take(synopsis), lines: std::mem::take(body) },
            });
        }
        names.clear();
        synopsis.clear();
        body.clear();
    };

    for raw in &lines {
        let line = *raw;
        let (mac, rest) = split_macro(line);
        match mac {
            Some("SH") | Some("SS") => {
                flush(&mut names, &mut synopsis, &mut body, &sh, &ss, skip);
                let title = troff_plain(&unquote_args(rest).join(" "));
                if mac == Some("SH") {
                    skip = matches!(title.as_str(), "NAME" | "SEE ALSO");
                    sh = title.clone();
                    ss.clear();
                } else {
                    ss = title.clone();
                }
                names = vec![title];
                is_item = false;
                want_term = false;
                depth = 0;
            }
            // What a top-level `.TP` starts is decided by its term: a bullet
            // or list number is a list inside the current topic, anything
            // else a topic of its own.
            Some("TP") if depth == 0 => want_term = true,
            Some("RS") => {
                depth += 1;
                body.push(line.to_string());
            }
            Some("RE") => {
                depth = depth.saturating_sub(1);
                body.push(line.to_string());
            }
            Some("PD") | Some("TH") | Some("so") | Some("\\\"") => {}
            _ if line.starts_with(".\\\"") || line == "." => {}
            _ if want_term => {
                want_term = false;
                let term = troff_plain(&macro_text(line));
                if is_list_marker(&term) && !names.is_empty() {
                    body.push(".TP".to_string());
                    body.push(line.to_string());
                    continue;
                }
                // `.PD 0` pairs: a term followed at once by another `.TP`
                // shares the next term's body — one topic, both names.
                let empty = !body.iter().any(|l| !l.starts_with('.') && !l.trim().is_empty());
                if !(is_item && empty) {
                    flush(&mut names, &mut synopsis, &mut body, &sh, &ss, skip);
                }
                is_item = true;
                let mut name = term_name(line);
                // A numbered entry (`1.` `Nested substitution`) is named by
                // the whole term, not the bold number alone.
                if is_list_marker(&name) && name != term {
                    name = term.clone();
                }
                if name.is_empty() {
                    // A continuation line of the previous term's synopsis.
                    if synopsis.is_empty() {
                        synopsis.extend(names.last().cloned());
                    }
                    synopsis.push(term);
                    continue;
                }
                if name != term {
                    synopsis.push(term);
                }
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            _ => body.push(line.to_string()),
        }
    }
    flush(&mut names, &mut synopsis, &mut body, &sh, &ss, skip);
    out
}

/// The name a `.TP` term documents. The manual sets the name in bold and
/// follows it with plain text — a builtin's synopsis
/// (`\fBtypeset\fP [ {+|-}AHU… ]`) or a widget's key bindings
/// (`\fBvi-forward-word\fP (unbound) (w)`) — so a leading bold run followed
/// by a space or nothing is the name. Any other term (`${name:-word}`, which
/// alternates bold and italic) is its own name.
///
/// The separating space may sit inside the bold run (`\fBprint \fP[ …`,
/// `\fBtypeset \fP\fB\-T\fP …`). A bold run of spaces alone continues the
/// previous term's synopsis on a new line; its name is empty.
fn term_name(line: &str) -> String {
    let full = macro_text(line);
    let plain = troff_plain(&full);
    let Some(rest) = full.strip_prefix("\\fB") else { return plain };
    let Some(end) = rest.find("\\f") else { return plain };
    let bold = &rest[..end];
    let after = &rest[end..];
    let after = after.strip_prefix("\\fP").or_else(|| after.strip_prefix("\\fR")).unwrap_or(after);
    if bold.ends_with(' ') || after.trim().is_empty() || after.starts_with(' ') {
        troff_plain(bold)
    } else {
        plain
    }
}

/// A `.TP` term that marks a list entry rather than naming something: a
/// bullet, a dash, or a list number (`1.`, `2)`, `(3)`).
fn is_list_marker(term: &str) -> bool {
    let t = term.trim_matches(|c| c == '(' || c == ')' || c == '.');
    matches!(term, "•" | "*" | "-" | "o") || (!t.is_empty() && t.chars().all(|c| c.is_ascii_digit()))
}

/// `.XX args` → (Some("XX"), "args"); a text line → (None, line).
fn split_macro(line: &str) -> (Option<&str>, &str) {
    let Some(rest) = line.strip_prefix('.').or_else(|| line.strip_prefix('\'')) else {
        return (None, line);
    };
    let rest = rest.trim_start();
    match rest.find(char::is_whitespace) {
        Some(i) => (Some(&rest[..i]), rest[i..].trim_start()),
        None => (Some(rest), ""),
    }
}

/// Macro arguments, honouring double quotes (`"two words"`).
fn unquote_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A font macro line (`.B x`, `.BR a b`) as inline troff text; any other
/// line unchanged.
fn macro_text(line: &str) -> String {
    let (mac, rest) = split_macro(line);
    let fonts: &[char] = match mac {
        Some("B") => &['B'],
        Some("I") => &['I'],
        Some("BR") => &['B', 'R'],
        Some("RB") => &['R', 'B'],
        Some("BI") => &['B', 'I'],
        Some("IB") => &['I', 'B'],
        Some("IR") => &['I', 'R'],
        Some("RI") => &['R', 'I'],
        Some(_) => return String::new(),
        None => return line.to_string(),
    };
    let args = unquote_args(rest);
    if fonts.len() == 1 {
        return format!("\\f{}{}\\fP", fonts[0], args.join(" "));
    }
    args.iter()
        .enumerate()
        .map(|(i, a)| format!("\\f{}{a}\\fP", fonts[i % 2]))
        .collect()
}

/// Inline troff to terminal text: font escapes to ANSI (bold → cyan, as code
/// spans are in the hover cards; italic → italic), character escapes to
/// their characters.
fn troff_inline(s: &str, t: &Theme) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('f') => match it.next() {
                Some('B') => out.push_str(t.c),
                Some('I') => out.push_str(t.i),
                Some(_) => out.push_str(t.n),
                None => {}
            },
            Some('(') => {
                let code: String = it.by_ref().take(2).collect();
                out.push_str(match code.as_str() {
                    "em" => "—",
                    "en" => "–",
                    "aq" => "'",
                    "dq" => "\"",
                    "lq" => "“",
                    "rq" => "”",
                    "oq" => "‘",
                    "cq" => "’",
                    "bu" => "•",
                    "co" => "©",
                    "ti" => "~",
                    "ha" => "^",
                    "rs" => "\\",
                    "mu" => "×",
                    "hy" => "-",
                    "ga" => "`",
                    _ => "",
                });
            }
            Some('*') => {
                // `\*(lq` / `\*q` string registers.
                if it.peek() == Some(&'(') {
                    it.next();
                    let code: String = it.by_ref().take(2).collect();
                    out.push_str(match code.as_str() {
                        "lq" => "“",
                        "rq" => "”",
                        _ => "",
                    });
                } else {
                    it.next();
                }
            }
            Some('-') => out.push('-'),
            Some('e') | Some('\\') => out.push('\\'),
            Some(' ') | Some('~') => out.push(' '),
            Some('&') | Some('c') | Some('%') | Some('|') | Some('^') => {}
            Some('\'') => out.push('\''),
            Some('`') => out.push('`'),
            Some('.') => out.push('.'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// Inline troff with every escape resolved and no styling.
fn troff_plain(s: &str) -> String {
    troff_inline(s, &Theme::new(false)).trim().to_string()
}

/// Render a topic's troff lines to wrapped terminal lines.
///
/// Fill-mode text is joined into paragraphs and word-wrapped; `.nf` blocks
/// and space-led lines keep their lines (examples, in green). Indentation
/// follows troff: a `.TP` term sits at the margin and its text one level in;
/// `.RS` moves the margin to the current text column (so an item's later
/// paragraphs line up with its first) and `.RE` restores it.
fn render_troff(lines: &[String], width: usize, t: &Theme) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut para = String::new();
    // The margin level, whether the text is an item's (one level in), and
    // the margins `.RS` saved.
    let mut base = 0usize;
    let mut in_item = false;
    let mut saved: Vec<(usize, bool)> = Vec::new();
    let mut nofill = false;
    let mut want_term = false;
    let mut url: Option<String> = None;

    let indent = |level: usize| "  ".to_string() + &"    ".repeat(level);
    fn flush(out: &mut Vec<String>, para: &mut String, ind: &str, width: usize) {
        let text = para.trim();
        if !text.is_empty() {
            for l in word_wrap(text, width.saturating_sub(ind.len()).max(20)) {
                out.push(format!("{ind}{l}"));
            }
        }
        para.clear();
    }
    fn blank(out: &mut Vec<String>) {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
    }

    for line in lines {
        let (mac, rest) = split_macro(line);
        let ind = indent(base + in_item as usize);
        match mac {
            Some("PP") | Some("LP") | Some("P") | Some("IP") => {
                flush(&mut out, &mut para, &ind, width);
                blank(&mut out);
                if mac == Some("IP") {
                    if let Some(tag) = unquote_args(rest).first().filter(|s| !s.is_empty()) {
                        para.push_str(&troff_inline(tag, t));
                        para.push(' ');
                    }
                }
            }
            Some("br") | Some("sp") => {
                flush(&mut out, &mut para, &ind, width);
                if mac == Some("sp") {
                    blank(&mut out);
                }
            }
            Some("nf") | Some("EX") => {
                flush(&mut out, &mut para, &ind, width);
                nofill = true;
            }
            Some("fi") | Some("EE") => nofill = false,
            Some("RS") => {
                flush(&mut out, &mut para, &ind, width);
                saved.push((base, in_item));
                base += in_item as usize;
                in_item = false;
            }
            Some("RE") => {
                flush(&mut out, &mut para, &ind, width);
                (base, in_item) = saved.pop().unwrap_or((0, false));
            }
            Some("TP") => {
                flush(&mut out, &mut para, &ind, width);
                blank(&mut out);
                in_item = false;
                want_term = true;
            }
            Some("UR") => url = Some(rest.trim().to_string()),
            Some("UE") => {
                if let Some(u) = url.take() {
                    para.push_str(&format!("<{u}>"));
                }
                para.push_str(rest);
                para.push(' ');
            }
            Some("B" | "I" | "BR" | "RB" | "BI" | "IB" | "IR" | "RI") | None => {
                let text = troff_inline(&macro_text(line), t);
                if want_term {
                    out.push(format!("{}{text}{}", indent(base), t.n));
                    in_item = true;
                    want_term = false;
                } else if nofill {
                    out.push(format!("{ind}{}{text}{}", t.g, t.n));
                } else if mac.is_none() && line.is_empty() {
                    flush(&mut out, &mut para, &ind, width);
                    blank(&mut out);
                } else if mac.is_none() && line.starts_with(' ') {
                    flush(&mut out, &mut para, &ind, width);
                    out.push(format!("{ind}{text}{}", t.n));
                } else {
                    para.push_str(&text);
                    para.push_str(t.n);
                    para.push(' ');
                }
            }
            Some(_) => {}
        }
    }
    flush(&mut out, &mut para, &indent(base + in_item as usize), width);
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out
}

/// A screenful of the book: its chapter, the lines shown, and the entries
/// that start on it.
struct Page {
    part: String,
    chapter: String,
    lines: Vec<String>,
    starts: Vec<usize>,
}

/// Consecutive runs of `key(entry)` with their topic counts, in book order.
fn runs<'a>(entries: &'a [Entry], key: impl Fn(&'a Entry) -> &'a str) -> Vec<(&'a str, usize)> {
    let mut out: Vec<(&str, usize)> = Vec::new();
    for e in entries {
        let k = key(e);
        match out.last_mut() {
            Some((c, n)) if *c == k => *n += 1,
            _ => out.push((k, 1)),
        }
    }
    out
}

/// Lay the rendered topics out on pages of `height` lines. A chapter always
/// starts a page; a topic taller than a page continues on the next ones.
fn build_pages(entries: &[Entry], height: usize, color: bool) -> Vec<Page> {
    let empty = |part: String, chapter: String| Page { part, chapter, lines: Vec::new(), starts: Vec::new() };
    let mut pages: Vec<Page> = Vec::new();
    let mut cur = empty(String::new(), String::new());
    for (i, e) in entries.iter().enumerate() {
        let text = e.render(color);
        let new_chapter = cur.chapter != e.chapter;
        let fits = cur.lines.len() + text.len() + 1 <= height;
        if !cur.lines.is_empty() && (new_chapter || !fits) {
            pages.push(std::mem::replace(&mut cur, empty(String::new(), String::new())));
        }
        cur.part = e.part.clone();
        cur.chapter = e.chapter.clone();
        cur.starts.push(i);
        if !cur.lines.is_empty() {
            cur.lines.push(String::new());
        }
        for line in text {
            if cur.lines.len() == height {
                let next = empty(cur.part.clone(), cur.chapter.clone());
                pages.push(std::mem::replace(&mut cur, next));
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
    ("}  {", "next / previous part"),
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
    i: &'static str,
    d: &'static str,
    b: &'static str,
    n: &'static str,
}

impl Theme {
    fn new(color: bool) -> Self {
        if color {
            Theme { c: "\x1b[36m", g: "\x1b[32m", i: "\x1b[3m", d: "\x1b[2m", b: "\x1b[1m", n: "\x1b[0m" }
        } else {
            Theme { c: "", g: "", i: "", d: "", b: "", n: "" }
        }
    }
}

/// The opening page: what the book holds, the keys, and its parts.
fn intro_lines(entries: &[Entry], t: &Theme) -> Vec<String> {
    let Theme { c, g, d, b, n, .. } = t;
    let parts = runs(entries, |e| e.part.as_str());
    let chapters = runs(entries, |e| e.chapter.as_str()).len();
    let mut out = vec![
        format!("  {d}>> THE ZSHRS REFERENCE // INTERACTIVE DOCUMENTATION <<{n}"),
        String::new(),
        format!("  {b}The zshrs manual and the whole zsh manual, every builtin,{n}"),
        format!("  {b}option, parameter, expansion, widget and completion style.{n}"),
        format!(
            "  {g}{}{n} {b}topics in {g}{chapters}{n} {b}chapters across {g}{}{n} {b}parts.{n}",
            entries.len(),
            parts.len()
        ),
        String::new(),
        format!("  {d}── KEYS ─────────────────────────────────────────────────{n}"),
    ];
    for (k, what) in KEYS {
        out.push(format!("  {c}{k:<20}{n} {what}"));
    }
    out.push(String::new());
    out.push(format!("  {d}── PARTS ────────────────────────────────────────────────{n}"));
    for (i, (part, count)) in parts.iter().enumerate() {
        out.push(format!("  {c}{:>2}.{n} {b}{part:<52}{n} {d}{count}{n}", i + 1));
    }
    out
}

/// `--toc`: every chapter grouped by part, with the page it starts on and,
/// with `with_topics`, each topic's page.
fn toc_lines(entries: &[Entry], pages: &[Page], t: &Theme, with_topics: bool) -> Vec<String> {
    let Theme { c, d, b, n, .. } = t;
    let mut out = Vec::new();
    let mut part = "";
    for (chapter, count) in runs(entries, |e| e.chapter.as_str()) {
        let first = pages.iter().position(|p| p.chapter == chapter).unwrap_or(0);
        let ep = &entries[pages[first].starts[0]].part;
        if ep != part {
            part = ep;
            out.push(format!("{b}{part}{n}"));
        }
        out.push(format!("  {chapter:<60} {d}{count} topics, page {}{n}", first + 2));
        if with_topics {
            for (pi, p) in pages.iter().enumerate().filter(|(_, p)| p.chapter == chapter) {
                for &ei in &p.starts {
                    out.push(format!("      {c}{:<40}{n} {d}{}{n}", entries[ei].name, pi + 2));
                }
            }
        }
    }
    out
}

/// Topics whose name, chapter or text contains `pat` (case-insensitive), as
/// (page number, topic name, chapter) — page 1 is the introduction.
fn search(entries: &[Entry], pages: &[Page], pat: &str) -> Vec<(usize, String)> {
    let pat = pat.to_lowercase();
    let mut out = Vec::new();
    for (pi, p) in pages.iter().enumerate() {
        for &ei in &p.starts {
            let e = &entries[ei];
            if e.text().to_lowercase().contains(&pat) {
                out.push((pi + 2, format!("{}  ({})", e.name, e.chapter)));
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
    println!("  {c}-l, --list{n}          every topic and its chapter, one per line");
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
        return print_topic(a, color);
    }

    let entries = entries();
    if matches!(first, Some("-l" | "--list")) {
        for e in &entries {
            println!("{}\t{}", e.name, e.chapter);
        }
        return 0;
    }
    let (rows, _) = term_size();
    let height = rows.saturating_sub(4).max(8);
    let pages = build_pages(&entries, height, color);
    let total = pages.len() + 1; // + the introduction

    match first {
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
            for (page, what) in &hits {
                println!("  {}{page:>5}{}  {what}", t.c, t.n);
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
/// A name only the manual documents (a widget, a style, an expansion flag)
/// prints its manual entry; a name the manual documents in several places
/// prints each.
fn print_topic(name: &str, color: bool) -> i32 {
    let card = crate::lsp::lookup_doc(if name == "(" { "LPAR" } else { name });
    if !card.is_empty() {
        if color {
            print!("{}", render_doc_card(name, &card, true));
        } else {
            print!("{card}");
            if !card.ends_with('\n') {
                println!();
            }
        }
        return 0;
    }
    let hits: Vec<Entry> = entries()
        .into_iter()
        .filter(|e| e.name == name || e.name.split(", ").any(|n| n == name))
        .collect();
    if hits.is_empty() {
        if name.starts_with('-') {
            eprintln!("zshrs: --docs: bad option: {name}");
        } else {
            eprintln!("zshrs: no docs for {name}");
        }
        return 1;
    }
    for (i, e) in hits.iter().enumerate() {
        if i > 0 {
            println!();
        }
        for l in e.render(color) {
            println!("{l}");
        }
    }
    0
}

/// Terminal rows and columns; 24x80 when there is no terminal. Read once:
/// without a terminal `crossterm` falls back to running `tput`, and the
/// renderer asks for the width once per topic — thousands of subprocesses
/// (`--docs --list` took 20 s) for a size that cannot change while the
/// pages are laid out.
fn term_size() -> (usize, usize) {
    static SIZE: std::sync::OnceLock<(usize, usize)> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| match crossterm::terminal::size() {
        Ok((cols, rows)) if rows > 0 && cols > 0 => (rows as usize, cols as usize),
        _ => (24, 80),
    })
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
    let chapter_of: fn(&Page) -> &str = |p| &p.chapter;
    let part_of: fn(&Page) -> &str = |p| &p.part;
    let next_run = |cur: usize, key: fn(&Page) -> &str| next_run(pages, cur, key);
    let prev_run = |cur: usize, key: fn(&Page) -> &str| prev_run(pages, cur, key);
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
            KeyCode::Char(']') => {
                cur = next_run(cur, chapter_of);
                page(cur)
            }
            KeyCode::Char('}') => {
                cur = next_run(cur, part_of);
                page(cur)
            }
            KeyCode::Char('[') => {
                cur = prev_run(cur, chapter_of);
                page(cur)
            }
            KeyCode::Char('{') => {
                cur = prev_run(cur, part_of);
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

/// The `key` (chapter or part) of book page `p`; the introduction (page 0)
/// has none.
fn page_key(pages: &[Page], p: usize, key: fn(&Page) -> &str) -> &str {
    if p == 0 { "" } else { key(&pages[p - 1]) }
}

/// The first page after `cur` whose `key` differs: the next chapter or part.
fn next_run(pages: &[Page], cur: usize, key: fn(&Page) -> &str) -> usize {
    let here = page_key(pages, cur, key);
    (cur + 1..=pages.len()).find(|&p| page_key(pages, p, key) != here).unwrap_or(cur)
}

/// The first page of `cur`'s chapter or part — or of the previous one when
/// `cur` is already its first page.
fn prev_run(pages: &[Page], cur: usize, key: fn(&Page) -> &str) -> usize {
    let mut p = cur;
    if p > 0 && page_key(pages, p - 1, key) != page_key(pages, p, key) {
        p -= 1;
    }
    let here = page_key(pages, p, key);
    while p > 0 && page_key(pages, p - 1, key) == here {
        p -= 1;
    }
    p
}
