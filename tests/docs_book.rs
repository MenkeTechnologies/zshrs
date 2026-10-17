//! `zshrs --docs`: the topic card and the reference book
//! (src/extensions/docs_book.rs). Every run here has no terminal on stdin,
//! so the book must print and exit — a test that blocked on a key read
//! would hang the suite, which is the regression
//! `bare_docs_without_a_tty_prints_the_introduction` guards.

use std::process::{Command, Output, Stdio};
use std::time::Duration;

/// Run `zshrs --docs ARGS` with no terminal. The output is collected on a
/// separate thread while the child runs — `--list` writes more than a pipe
/// holds, so waiting for exit before reading would deadlock — and a child
/// still running after two minutes fails the test instead of hanging it.
fn docs(args: &[&str]) -> Output {
    let child = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .arg("--docs")
        .args(args)
        .env_remove("FORCE_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn zshrs");
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(Duration::from_secs(120)) {
        Ok(out) => out.expect("output"),
        Err(_) => {
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            panic!("zshrs --docs {args:?} did not exit without a terminal");
        }
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// The book is the manuals and the language reference: every man page the
/// binary carries is a part, in `zshall.1`'s order after zshrs's own, and the
/// reference parts sit between them; no project guide is in it.
#[test]
fn the_book_holds_every_manual_in_order() {
    let entries = zsh::docs_book::entries();
    let mut parts: Vec<&str> = Vec::new();
    for e in &entries {
        if parts.last() != Some(&e.part.as_str()) {
            parts.push(&e.part);
        }
    }
    let starts = |p: &str| parts.iter().position(|x| x.starts_with(p)).unwrap_or_else(|| panic!("no part {p}: {parts:?}"));
    // zshrs first, zsh's manual in zshall.1's .so order after it.
    let order = [
        "zshrs -", "zshrsall", "Language reference", "Reference indexes", "zsh -", "zshroadmap", "zshmisc", "zshexpn",
        "zshparam", "zshoptions", "zshbuiltins", "zshzle", "zshcompwid", "zshcompsys", "zshcompctl",
        "zshmodules", "zshcalsys", "zshtcpsys", "zshzftpsys", "zshcontrib",
    ];
    let idx: Vec<usize> = order.iter().map(|p| starts(p)).collect();
    assert!(idx.windows(2).all(|w| w[0] < w[1]), "parts out of order: {parts:?}");
    // Parts are contiguous: no part reappears after another began.
    let mut seen = std::collections::HashSet::new();
    for p in &parts {
        assert!(seen.insert(*p), "part {p} is split");
    }
    // Things only the manual documents are topics.
    for (name, chapter) in [
        ("list-colors", "zshcompsys"),
        ("vi-forward-word", "zshzle"),
        ("TRAPNAL", "zshmisc"),
        ("AUTO_CD", "zshoptions"),
        ("typeset", "zshbuiltins"),
    ] {
        assert!(
            entries.iter().any(|e| e.name.split(", ").any(|n| n == name) && e.chapter.starts_with(chapter)),
            "{name} is not a topic in {chapter}"
        );
    }
    // The reference page is in the book; the repository guides are not.
    for (name, chapter) in [("if … then … fi", "Control Flow"), ("pmap", "Extension Builtin Index")] {
        assert!(entries.iter().any(|e| e.name == name && e.chapter == chapter), "{name} is not a topic in {chapter}");
    }
    assert!(
        !entries.iter().any(|e| e.part.starts_with("README") || e.part.contains("RFC") || e.part.contains("INVENTIONS")),
        "a project guide is in the book"
    );
    // A bullet or list number is part of a topic, never a topic of its own.
    assert!(
        !entries.iter().any(|e| matches!(e.name.as_str(), "•" | "1." | "2.")),
        "a list marker became a topic"
    );
}

/// `--list` is exactly the book's entries, `name<TAB>chapter`, one per line.
#[test]
fn list_is_the_books_entries() {
    let o = docs(&["--list"]);
    assert!(o.status.success());
    let listed: Vec<String> = stdout(&o).lines().map(str::to_string).collect();
    let built: Vec<String> =
        zsh::docs_book::entries().into_iter().map(|e| format!("{}\t{}", e.name, e.chapter)).collect();
    assert_eq!(listed, built);
}

/// `--toc` lists every chapter once, grouped under its part, and the
/// chapters' topic counts add up to the whole book.
#[test]
fn toc_chapters_cover_every_topic() {
    let o = docs(&["--toc"]);
    assert!(o.status.success());
    let text = stdout(&o);
    let counts: Vec<usize> = text
        .lines()
        .filter(|l| l.starts_with("  ") && !l.starts_with("      "))
        .filter_map(|l| {
            let tail = l.rsplit_once(" topics, page ")?.0;
            tail.split_whitespace().last()?.parse().ok()
        })
        .collect();
    let entries = zsh::docs_book::entries();
    assert_eq!(counts.iter().sum::<usize>(), entries.len());
    let chapters: std::collections::HashSet<&str> = entries.iter().map(|e| e.chapter.as_str()).collect();
    assert_eq!(counts.len(), chapters.len(), "a chapter is listed twice or missing");
    assert!(text.lines().any(|l| l.starts_with("zshexpn - ")), "no zshexpn part heading");
}

/// `--search` finds a topic and reports the page it is on; that page printed
/// on its own carries the topic.
#[test]
fn search_reports_the_page_a_topic_is_on() {
    let o = docs(&["--search", "list-colors"]);
    assert!(o.status.success());
    let hit = stdout(&o)
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some("list-colors"))
        .map(str::to_string)
        .expect("list-colors in the search results");
    let page = hit.split_whitespace().next().unwrap();
    let p = docs(&[page]);
    assert!(p.status.success());
    assert!(stdout(&p).contains("list-colors"), "page {page} does not show list-colors");

    let none = docs(&["--search", "no-such-topic-anywhere-zz"]);
    assert_eq!(none.status.code(), Some(1));
}

/// Page 1 is the introduction; a page past the end is an error.
#[test]
fn pages_print_and_out_of_range_fails() {
    let o = docs(&["1"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("THE ZSHRS REFERENCE"));
    let o = docs(&["999999"]);
    assert_eq!(o.status.code(), Some(1));
}

/// Bare `--docs` with no terminal prints the introduction — the topic count
/// and every part — and exits 0 instead of waiting for keys.
#[test]
fn bare_docs_without_a_tty_prints_the_introduction() {
    let o = docs(&[]);
    assert!(o.status.success());
    let text = stdout(&o);
    assert!(text.contains("THE ZSHRS REFERENCE"));
    let entries = zsh::docs_book::entries();
    assert!(text.contains(&entries.len().to_string()), "intro does not state the topic count");
    for part in ["zshexpn - zsh expansion and substitution", "zshzle - zsh command line editor"] {
        assert!(text.contains(part), "intro does not list {part}");
    }
}

/// A topic the LSP documents prints its card — the raw markdown without
/// colour (the editor popup and `| glow` need the `**name**` heading) — and
/// a name only the manual documents prints the manual entry. `(` and `-`
/// are topics, not options.
#[test]
fn topics_print_their_card_or_manual_entry() {
    let o = docs(&["typeset", "--color", "never"]);
    assert!(o.status.success());
    assert_eq!(stdout(&o), format!("{}\n", zsh::lsp::lookup_doc("typeset").trim_end_matches('\n')));
    let o = docs(&["typeset", "--color", "always"]);
    assert!(stdout(&o).contains("\x1b[36mtypeset\x1b[0m"));
    for topic in ["(", "-"] {
        let o = docs(&[topic, "--color", "never"]);
        assert!(o.status.success(), "{topic} is not a topic");
        assert!(stdout(&o).contains("_zsh keyword_"));
    }
    let o = docs(&["list-colors", "--color", "never"]);
    assert!(o.status.success(), "list-colors (a compsys style) has no entry");
    let text = stdout(&o);
    assert!(text.contains("zshcompsys: COMPLETION SYSTEM CONFIGURATION › Standard Styles"));
    assert!(text.contains("ZLS_COLORS"));
}

/// A miss states the error and exits 1 — no "did you mean" suggestion. An
/// unknown dashed word is a bad option.
#[test]
fn unknown_topic_and_option_fail_without_suggestions() {
    let o = docs(&["no_such_topic_zz"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&o.stderr), "zshrs: no docs for no_such_topic_zz\n");
    let o = docs(&["--bogus"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&o.stderr), "zshrs: --docs: bad option: --bogus\n");
}
