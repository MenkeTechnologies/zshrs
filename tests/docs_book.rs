//! `zshrs --docs`: the topic card and the reference book
//! (src/extensions/docs_book.rs). Every run here has no terminal on stdin,
//! so the book must print and exit — a test that blocked on a key read
//! would hang the suite, which is the regression `bare_docs_without_a_tty`
//! guards.

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn docs(args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .arg("--docs")
        .args(args)
        .env_remove("FORCE_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn zshrs");
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().expect("wait").is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("zshrs --docs {args:?} did not exit without a terminal");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().expect("output")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// `--list` is exactly the book's entries: one line per topic, no
/// duplicates, the same count the library builds.
#[test]
fn list_is_the_books_entries() {
    let o = docs(&["--list"]);
    assert!(o.status.success());
    let listed: Vec<String> = stdout(&o).lines().map(str::to_string).collect();
    let built: Vec<String> = zsh::docs_book::entries().into_iter().map(|e| e.name).collect();
    assert_eq!(listed, built);
    let mut uniq = listed.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), listed.len(), "a topic is listed twice");
    for want in ["typeset", "if", "&&", "AUTO_CD", "zls", "("] {
        assert!(listed.iter().any(|n| n == want), "{want} missing from --list");
    }
}

/// `--toc` names the six chapters in book order, and their topic counts add
/// up to the whole list.
#[test]
fn toc_chapters_cover_every_topic() {
    let o = docs(&["--toc"]);
    assert!(o.status.success());
    let text = stdout(&o);
    let chapters: Vec<(String, usize)> = text
        .lines()
        .filter(|l| !l.starts_with("      "))
        .filter_map(|l| {
            let (name, rest) = l.trim().split_once("  ")?;
            let count = rest.split_whitespace().next()?.parse().ok()?;
            Some((name.trim().to_string(), count))
        })
        .collect();
    let names: Vec<&str> = chapters.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["Reserved words", "Operators", "Builtins", "zshrs builtins", "Options", "Special parameters"]
    );
    let total: usize = chapters.iter().map(|(_, c)| c).sum();
    assert_eq!(total, zsh::docs_book::entries().len());
}

/// `--search` finds a topic by name and reports the page it is on; that page
/// printed on its own carries the topic.
#[test]
fn search_reports_the_page_a_topic_is_on() {
    let o = docs(&["--search", "AUTO_CD"]);
    assert!(o.status.success());
    let hit = stdout(&o)
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some("AUTO_CD"))
        .map(str::to_string)
        .expect("AUTO_CD in the search results");
    let page = hit.split_whitespace().next().unwrap();
    let p = docs(&[page]);
    assert!(p.status.success());
    assert!(stdout(&p).contains("AUTO_CD"), "page {page} does not show AUTO_CD");

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

/// Bare `--docs` with no terminal prints the introduction and exits 0
/// instead of waiting for keys.
#[test]
fn bare_docs_without_a_tty_prints_the_introduction() {
    let o = docs(&[]);
    assert!(o.status.success());
    let text = stdout(&o);
    assert!(text.contains("THE ZSHRS REFERENCE"));
    let n = zsh::docs_book::entries().len().to_string();
    assert!(text.contains(&n), "intro does not state the {n} topics");
}

/// A topic card without colour is the raw markdown `lookup_doc` returns —
/// the editor popup and `| glow` depend on the `**name**` heading — and
/// `(` / `-` are topics, not options.
#[test]
fn topics_print_their_card() {
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
