//! `zshrs --docs` — the reference book.
//!
//! zshrs-original. With a topic, `zshrs --docs NAME` prints the hover card
//! the LSP returns for NAME (`lsp::lookup_doc`), or, for a name only the
//! manual documents (a ZLE widget, a completion style, an expansion flag),
//! that manual entry. Without one it opens the whole reference as a paged
//! book in the terminal — the interface `stryke docs` has: an introduction,
//! chapters, a table of contents, search and vim-style keys.
//!
//! The book is the reference, not a second copy of it: zshrs's own pages
//! (`man/man1/zshrs.1`, `zshrsall.1`), the language reference
//! (`docs/reference.html`, compiled in) and every page of zsh's manual
//! (`vendor/zsh/man1`, read from `bundled_docs` in `zshall.1`'s order).
//! Each man page and each reference part is a part, each `.SH` / `.SS`
//! section or reference chapter a chapter, and each `.TP` item or reference
//! entry a topic; a section's opening prose is a topic of its own. Project
//! guides, plans and design notes are not part of the book.
//!
//! The interactive book runs only when stdin and stdout are both terminals
//! and `ZSHRS_NO_TTY` is unset; anything else prints and exits, so a pipe,
//! CI job or editor can never block on a key read.

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
    // zshrs's own pages first, then the language reference, then zsh's manual.
    while let Some((page, _)) = pages.peek() {
        if !page.starts_with("zshrs") {
            break;
        }
        let (page, text) = pages.next().unwrap();
        out.extend(parse_man(&page, &text));
    }
    out.extend(parse_reference_html(REFERENCE_HTML));
    for (page, text) in pages {
        out.extend(parse_man(&page, &text));
    }
    out
}

/// The language reference, `docs/reference.html`, compiled in so the book
/// always matches the build.
const REFERENCE_HTML: &str = include_str!("../../docs/reference.html");

/// Split the reference page into topics: each `<section id="ch-…">` is a
/// chapter (the numbered ones form the language part, the `ch-lsp-` ones the
/// index part), each `doc-entry` article a topic, and the prose before a
/// chapter's first article a topic named after the chapter.
fn parse_reference_html(html: &str) -> Vec<Entry> {
    const SECTION: &str = "<section class=\"tutorial-section\" id=\"ch-";
    const ARTICLE: &str = "<article class=\"doc-entry\"";
    let mut out = Vec::new();
    for section in html.split(SECTION).skip(1) {
        let section = section.split("</section>").next().unwrap_or("");
        let id = section.split('"').next().unwrap_or("");
        let part = if id.starts_with("lsp-") { "Reference indexes" } else { "Language reference" };
        let Some(chapter) = tag_inner(section, "h2").map(|h| html_text(&h)) else { continue };
        let mut pieces = section.split(ARTICLE);
        let intro = pieces.next().unwrap_or("");
        let intro = html_to_markdown(intro.split_once("</h2>").map_or(intro, |(_, rest)| rest));
        let mut push = |name: String, text: String| {
            if !text.trim().is_empty() {
                out.push(Entry {
                    part: part.to_string(),
                    chapter: chapter.clone(),
                    body: Body::Card(format!("**{name}** — _{chapter}_\n\n{}", text.trim())),
                    name,
                });
            }
        };
        push(chapter.clone(), intro);
        for article in pieces {
            let article = article.split("</article>").next().unwrap_or("");
            let Some(name) = tag_inner(article, "h3").map(|h| html_text(&h)) else { continue };
            let name = name.trim_start_matches('#').trim_end_matches('¶').trim().to_string();
            let body = article.split_once("</h3>").map_or("", |(_, rest)| rest);
            push(name, html_to_markdown(body));
        }
    }
    out
}

/// The inner HTML of the first `<tag …>…</tag>` in `s`.
fn tag_inner(s: &str, tag: &str) -> Option<String> {
    let open = s.find(&format!("<{tag}"))?;
    let start = open + s[open..].find('>')? + 1;
    let end = start + s[start..].find(&format!("</{tag}>"))?;
    Some(s[start..end].to_string())
}

/// Markup removed and entities decoded, whitespace collapsed.
fn html_text(s: &str) -> String {
    html_to_markdown(s).split_whitespace().collect::<Vec<_>>().join(" ").replace('`', "").replace("**", "")
}

/// HTML to the markdown subset `render_doc_card` draws: code spans and
/// fences, bold, emphasis and bullets; every other tag is dropped.
fn html_to_markdown(html: &str) -> String {
    let mut out = String::new();
    let mut in_pre = false;
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&decode_entities(&rest[..lt]));
        rest = &rest[lt..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace()).next().unwrap_or("");
        let closing = tag.starts_with('/');
        match (name, closing) {
            ("pre", false) => {
                in_pre = true;
                out.push_str("\n```\n");
            }
            ("pre", true) => {
                in_pre = false;
                out.push_str("\n```\n");
            }
            ("code", _) if !in_pre => out.push('`'),
            ("strong" | "b", _) => out.push_str("**"),
            ("em" | "i", _) => out.push('_'),
            ("li", false) => out.push_str("\n- "),
            ("p" | "ul" | "ol" | "div", true) => out.push_str("\n\n"),
            ("br", _) => out.push('\n'),
            _ => {}
        }
    }
    out.push_str(&decode_entities(rest));
    out.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").replace("\n\n\n", "\n\n")
}

fn decode_entities(s: &str) -> String {
    [
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&nbsp;", " "),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
        ("&rarr;", "→"),
        ("&larr;", "←"),
        ("&rsquo;", "’"),
        ("&lsquo;", "‘"),
        ("&hellip;", "…"),
        ("&middot;", "·"),
        ("&amp;", "&"),
    ]
    .iter()
    .fold(s.to_string(), |acc, (from, to)| acc.replace(from, to))
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

/// A screenful of the book: its chapter, the lines shown, the entries that
/// start on it, and every entry with text on it (a topic continued from the
/// page before included).
struct Page {
    part: String,
    chapter: String,
    lines: Vec<String>,
    starts: Vec<usize>,
    shown: Vec<usize>,
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
    let empty =
        |part: String, chapter: String| Page { part, chapter, lines: Vec::new(), starts: Vec::new(), shown: Vec::new() };
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
        cur.shown.push(i);
        if !cur.lines.is_empty() {
            cur.lines.push(String::new());
        }
        for line in text {
            if cur.lines.len() == height {
                let mut next = empty(cur.part.clone(), cur.chapter.clone());
                next.shown.push(i);
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

/// The keys the introduction lists, `/` separating alternatives.
const KEYS: &[(&str, &str)] = &[
    ("j / n / space", "next page"),
    ("k / p", "previous page"),
    ("] / [", "next / previous chapter"),
    ("} / {", "next / previous part"),
    ("d / u", "forward / back 5 pages"),
    ("g / G", "first / last page"),
    ("t", "table of contents"),
    ("/", "search all pages"),
    (":", "jump to page number"),
    ("r", "random page"),
    ("?", "full keybinding help"),
    ("q", "quit"),
];

/// Every key, grouped, for `?` and `--docs -h`.
const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigation",
        &[
            ("j n l space enter", "next page"),
            ("k p h", "previous page"),
            ("d", "forward 5 pages"),
            ("u", "back 5 pages"),
            ("g 0", "first page"),
            ("G $", "last page"),
            ("] [", "next / previous chapter"),
            ("} {", "next / previous part"),
        ],
    ),
    (
        "Search & Jump",
        &[
            ("/", "search all pages"),
            (":", "go to page number"),
            ("t", "table of contents"),
            ("r", "random page"),
        ],
    ),
    ("Other", &[("?", "this help"), ("q esc", "quit")]),
];

/// The footer's key line, as (keys, what); trailing items are dropped on a
/// terminal too narrow for them.
const FOOTER_KEYS: &[(&str, &str)] = &[
    ("j/n", "next"),
    ("k/p", "prev"),
    ("d/u", "±5"),
    ("]/[", "chapter"),
    ("t", "toc"),
    ("/", "search"),
    (":", "num"),
    ("r", "rand"),
    ("?", "help"),
    ("q", "quit"),
    ("}/{", "part"),
];

/// Rows of chrome around a page: a blank line, the logo and the status box
/// above it; the rule, the key line and the `>>>` prompt below it.
const HEADER_ROWS: usize = 10;
const FOOTER_ROWS: usize = 3;

struct Theme {
    c: &'static str,
    g: &'static str,
    m: &'static str,
    i: &'static str,
    d: &'static str,
    b: &'static str,
    n: &'static str,
}

impl Theme {
    fn new(color: bool) -> Self {
        if color {
            Theme {
                c: "\x1b[36m",
                g: "\x1b[32m",
                m: "\x1b[35m",
                i: "\x1b[3m",
                d: "\x1b[2m",
                b: "\x1b[1m",
                n: "\x1b[0m",
            }
        } else {
            Theme { c: "", g: "", m: "", i: "", d: "", b: "", n: "" }
        }
    }

    fn color(&self) -> bool {
        !self.n.is_empty()
    }
}

/// The status box's inner width: 74 columns, less on a narrow terminal.
fn box_width() -> usize {
    74.min(term_width().saturating_sub(4)).max(20)
}

/// `s` cut to `max` characters, the cut marked with `…`.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        s.chars().take(max - 1).collect::<String>() + "…"
    }
}

/// The head of every screen: a blank line, the ZSHRS logo (`zbanner`'s), and
/// `status` in a box.
fn masthead(status: &str, t: &Theme) -> Vec<String> {
    let w = box_width();
    let mut out = vec![String::new()];
    for (sgr, line) in crate::banner::LOGO {
        out.push(if t.color() { format!("\x1b[{sgr}m{line}\x1b[0m") } else { line.to_string() });
    }
    let pad = " ".repeat(w.saturating_sub(visible_width(status)));
    out.push(format!(" {}┌{}┐{}", t.d, "─".repeat(w), t.n));
    out.push(format!(" {}│{}{status}{pad}{}│{}", t.d, t.n, t.d, t.n));
    out.push(format!(" {}└{}┘{}", t.d, "─".repeat(w), t.n));
    out
}

/// The status box's text: `left` in green, `middle` in cyan, `right` in
/// magenta, `//` between them, cut to fit the box.
fn status_line(left: &str, middle: &str, right: &str, t: &Theme) -> String {
    let room = box_width().saturating_sub(left.chars().count() + 11);
    let (m, r) = (middle.chars().count(), right.chars().count());
    let right = if m + r <= room { right.to_string() } else { clip(right, r.min(room.saturating_sub(m).max(room / 2))) };
    let middle = clip(middle, room.saturating_sub(right.chars().count()));
    let Theme { c, g, m, d, n, .. } = t;
    format!(" {g}{left}{n}  {d}//{n} {c}{middle}{n}  {d}//{n} {m}{right}{n}")
}

/// `── LABEL ───…` across the page.
fn section_rule(label: &str, t: &Theme) -> String {
    let trail = box_width().saturating_sub(2 + label.chars().count());
    format!("  {}── {label} {}{}", t.d, "─".repeat(trail), t.n)
}

/// `j / n / space` with each key cyan and the separators plain.
fn keycap(keys: &str, t: &Theme) -> String {
    if keys.len() == 1 {
        return format!("{}{keys}{}", t.c, t.n);
    }
    keys.split('/')
        .map(|k| {
            let (lead, k, trail) = (&k[..k.len() - k.trim_start().len()], k.trim(), &k[k.trim_end().len()..]);
            format!("{lead}{}{k}{}{trail}", t.c, t.n)
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The two lines under every screen: a rule and the keys that fit.
fn footer(t: &Theme) -> [String; 2] {
    let room = term_width().saturating_sub(3);
    let mut keys = String::new();
    let mut used = 0usize;
    for (k, what) in FOOTER_KEYS {
        let item = if *k == ":" { format!("{}:{}{what}", t.c, t.n) } else { format!("{} {what}", keycap(k, t)) };
        let w = visible_width(&item) + if used == 0 { 0 } else { 2 };
        if used + w > room {
            break;
        }
        if used > 0 {
            keys.push_str("  ");
        }
        keys.push_str(&item);
        used += w;
    }
    [format!("  {}{}{}", t.d, "─".repeat(box_width() + 2), t.n), format!("  {keys}")]
}

/// The `>>>` prompt on the last row.
fn prompt(t: &Theme) -> String {
    format!("  {}>>>{} ", t.d, t.n)
}

/// The opening page: what the book holds, the keys, and its parts. With
/// `room`, the part list stops where the page would overflow and one line
/// counts the rest; without it every part is listed.
fn intro_lines(entries: &[Entry], t: &Theme, room: Option<usize>) -> Vec<String> {
    let Theme { c, g, d, b, n, .. } = t;
    let parts = runs(entries, |e| e.part.as_str());
    let chapters = runs(entries, |e| e.chapter.as_str()).len();
    let mut out = vec![
        format!("  {d}>> THE ZSHRS REFERENCE // INTERACTIVE REFERENCE SYSTEM <<{n}"),
        String::new(),
        format!("  {b}A comprehensive reference for zshrs and the whole zsh manual: every{n}"),
        format!("  {b}builtin, option, parameter, expansion, widget and completion style.{n}"),
        format!(
            "  {g}{}{n} {b}topics across {g}{chapters}{n} {b}chapters in {g}{}{n} {b}parts.{n}",
            entries.len(),
            parts.len()
        ),
        String::new(),
        section_rule("GETTING STARTED", t),
        String::new(),
    ];
    for (k, what) in KEYS {
        out.push(format!("  {}{}{what}", keycap(k, t), " ".repeat(21usize.saturating_sub(k.len()))));
    }
    out.push(String::new());
    out.push(section_rule("PARTS", t));
    // The blank line and the closing line stay on the page.
    let fit = room.map_or(usize::MAX, |r| r.saturating_sub(out.len() + 2));
    let listed = if parts.len() <= fit { parts.len() } else { fit.saturating_sub(1) };
    for (i, (part, count)) in parts.iter().take(listed).enumerate() {
        out.push(format!("  {c}{:>2}.{n} {b}{:<52}{n} {d}{count} topics{n}", i + 1, clip(part, 52)));
    }
    if listed < parts.len() {
        let rest: usize = parts[listed..].iter().map(|(_, n)| n).sum();
        let more = format!("+ {} more parts", parts.len() - listed);
        out.push(format!("  {d}…{n}  {b}{more:<52}{n} {d}{rest} topics{n}"));
    }
    out.push(String::new());
    out.push(format!("  {d}press {c}t{d} for full table of contents, {c}j{d} or {c}space{d} to begin >>>{n}"));
    out
}

/// The `?` screen: every key, grouped.
fn help_lines(t: &Theme) -> Vec<String> {
    let Theme { c, d, b, n, .. } = t;
    let mut out = vec![section_rule("KEYBINDINGS", t)];
    for (group, keys) in HELP {
        out.push(String::new());
        out.push(format!("  {b}{group}{n}"));
        for (k, what) in *keys {
            out.push(format!("  {c}{k:<20}{n} {d}{what}{n}"));
        }
    }
    out.push(String::new());
    out.push(format!("  {d}press any key to return{n}"));
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

/// The status box over the table of contents.
fn toc_status(entries: &[Entry], total: usize, t: &Theme) -> String {
    status_line("TABLE OF CONTENTS", &format!("{} topics, {total} pages", entries.len()), "The zshrs Reference", t)
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
    let Theme { c, g, d, b, n, .. } = t;
    let row = |k: &str, what: &str| println!("  {c}{k:<36}{n}{d}// {what}{n}");
    for l in masthead(&status_line("STATUS: ONLINE", &format!("SIGNAL: {g}████████{d}░░"), "ZSHRS DOCS", t), t) {
        println!("{l}");
    }
    println!("  {d}>> THE ZSHRS REFERENCE // INTERACTIVE REFERENCE SYSTEM <<{n}");
    println!();
    println!("  {b}USAGE:{n} zshrs --docs {d}[OPTIONS] [PAGE|TOPIC]{n}");
    println!();
    println!("{}", section_rule("OPTIONS", t));
    row("-h, --help", "Show this help");
    row("-t, --toc", "Table of contents with every topic's page");
    row("-s, --search <pattern>", "Topics whose name or text contains the pattern");
    row("-l, --list", "Every topic and its chapter, one per line");
    row("TOPIC", "Print a topic's entry (zshrs --docs typeset)");
    row("PAGE", "Print a page");
    println!();
    println!("{}", section_rule("NAVIGATION (vim-style)", t));
    for (_, keys) in HELP {
        for (k, what) in *keys {
            row(k, what);
        }
    }
    println!();
    println!("{}", section_rule("EXAMPLES", t));
    row("zshrs --docs", "open the book at the introduction");
    row("zshrs --docs --toc", "table of contents");
    row("zshrs --docs 42", "print page 42");
    row("zshrs --docs typeset", "the typeset entry");
    row("zshrs --docs --search list-colors", "find list-colors");
    println!();
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
    let height = rows.saturating_sub(HEADER_ROWS + FOOTER_ROWS).max(4);
    let pages = build_pages(&entries, height, color);
    let total = pages.len() + 1; // + the introduction

    match first {
        Some("-t" | "--toc") => {
            for l in masthead(&toc_status(&entries, total, &t), &t) {
                println!("{l}");
            }
            println!();
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
    let interactive = first.is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::env::var_os("ZSHRS_NO_TTY").is_none();
    if !interactive {
        let lines = if start == 0 { intro_lines(&entries, &t, None) } else { pages[start - 1].lines.clone() };
        for l in lines {
            println!("{l}");
        }
        return 0;
    }
    let intro = intro_lines(&entries, &t, Some(height));
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

/// What the screen shows: a page of the book, or a list (the table of
/// contents, search results, the keys) scrolled to `top`. `any_key` lists
/// close on any key that does not scroll them; the others on q, Esc or Enter.
enum View {
    Page(usize),
    List { status: String, lines: Vec<String>, top: usize, any_key: bool },
}

/// One screen: the masthead over `status`, `body` (at most `height` lines),
/// and the footer pinned to the bottom rows, the cursor left at `>>>`.
fn draw(out: &mut impl Write, status: &str, body: &[String], height: usize, t: &Theme) -> std::io::Result<()> {
    write!(out, "\x1b[H\x1b[2J")?;
    for line in masthead(status, t).iter().chain(body.iter().take(height)) {
        write!(out, "{line}\r\n")?;
    }
    let rows = term_size().0.max(HEADER_ROWS + FOOTER_ROWS + 1);
    let [rule, keys] = footer(t);
    write!(out, "\x1b[{};1H{rule}\r\n{keys}\r\n{}", rows - 2, prompt(t))?;
    out.flush()
}

/// Read a line after the `>>>` prompt and `lead` (`/` or `:`); None on Esc.
fn read_line(out: &mut impl Write, lead: &str, t: &Theme) -> std::io::Result<Option<String>> {
    use crossterm::event::{read, Event, KeyCode, KeyEventKind};
    let row = term_size().0.max(HEADER_ROWS + FOOTER_ROWS + 1);
    let mut s = String::new();
    loop {
        write!(out, "\x1b[{row};1H\x1b[2K{}{}{lead}{}{s}", prompt(t), t.c, t.n)?;
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

/// The page a `:N` answer names, 0-based; None for anything else.
fn page_number(answer: Option<String>, total: usize) -> Option<usize> {
    answer?.trim().parse::<usize>().ok().filter(|n| (1..=total).contains(n)).map(|n| n - 1)
}

fn browse(entries: &[Entry], pages: &[Page], intro: &[String], start: usize, height: usize, t: &Theme) -> std::io::Result<()> {
    use crossterm::event::{read, Event, KeyCode, KeyEventKind, KeyModifiers};
    let _screen = Screen::enter()?;
    let mut out = std::io::stdout();
    let total = pages.len() + 1;
    let mut cur = start;
    let mut view = View::Page(cur);
    // A chapter's first page; page 0 is the introduction.
    let chapter_of: fn(&Page) -> &str = |p| &p.chapter;
    let part_of: fn(&Page) -> &str = |p| &p.part;
    let next_run = |cur: usize, key: fn(&Page) -> &str| next_run(pages, cur, key);
    let prev_run = |cur: usize, key: fn(&Page) -> &str| prev_run(pages, cur, key);
    loop {
        match &view {
            View::Page(0) => draw(&mut out, &status_line(&format!("{:>3}/{total}", 1), "", "Introduction", t), intro, height, t)?,
            View::Page(p) => {
                let page = &pages[p - 1];
                let names: Vec<&str> = page.shown.iter().map(|&i| entries[i].name.as_str()).collect();
                let mut topics = names.iter().take(3).copied().collect::<Vec<_>>().join(", ");
                if names.len() > 3 {
                    topics += &format!(" +{}", names.len() - 3);
                }
                let status = status_line(&format!("{:>3}/{total}", p + 1), &topics, &page.chapter, t);
                draw(&mut out, &status, &page.lines, height, t)?;
            }
            View::List { status, lines, top, .. } => draw(&mut out, status, &lines[*top..], height, t)?,
        }
        let Event::Key(k) = read()? else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }
        if let View::List { lines, top, any_key, .. } = &mut view {
            let bottom = lines.len().saturating_sub(height);
            match k.code {
                KeyCode::Char('j') | KeyCode::Down => *top = (*top + 1).min(bottom),
                KeyCode::Char('k') | KeyCode::Up => *top = top.saturating_sub(1),
                KeyCode::Char(' ' | 'd') | KeyCode::PageDown => *top = (*top + height).min(bottom),
                KeyCode::Char('u') | KeyCode::PageUp => *top = top.saturating_sub(height),
                KeyCode::Char('g') | KeyCode::Home => *top = 0,
                KeyCode::Char('G') | KeyCode::End => *top = bottom,
                KeyCode::Char(':') => {
                    if let Some(p) = page_number(read_line(&mut out, ":", t)?, total) {
                        cur = p;
                        view = View::Page(cur);
                    }
                }
                KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => view = View::Page(cur),
                _ if *any_key => view = View::Page(cur),
                _ => {}
            }
            continue;
        }
        let page = |p: usize| View::Page(p.min(total - 1));
        view = match k.code {
            KeyCode::Char('q' | 'Q') | KeyCode::Esc => return Ok(()),
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
            KeyCode::Char('t') => View::List {
                status: toc_status(entries, total, t),
                lines: toc_lines(entries, pages, t, false),
                top: 0,
                any_key: false,
            },
            KeyCode::Char('?') => View::List {
                status: status_line("KEYBINDINGS", "vim-style", "The zshrs Reference", t),
                lines: help_lines(t),
                top: 0,
                any_key: true,
            },
            KeyCode::Char('/') => match read_line(&mut out, "/", t)? {
                Some(pat) if !pat.is_empty() => {
                    let hits = search(entries, pages, &pat);
                    if let [(p, _)] = hits.as_slice() {
                        cur = p - 1;
                        page(cur)
                    } else {
                        View::List {
                            status: status_line("SEARCH", &pat, &format!("{} topics", hits.len()), t),
                            lines: hits.iter().map(|(p, name)| format!("  {}{p:>5}{}  {name}", t.c, t.n)).collect(),
                            top: 0,
                            any_key: false,
                        }
                    }
                }
                _ => page(cur),
            },
            KeyCode::Char(':') => {
                if let Some(p) = page_number(read_line(&mut out, ":", t)?, total) {
                    cur = p;
                }
                page(cur)
            }
            _ => page(cur),
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
