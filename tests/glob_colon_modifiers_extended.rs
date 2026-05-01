//! Extended colon-modifier coverage — `:a :A :c :l :u :q :Q :P :S`.
//! Direct port of zsh/Src/subst.c:4733-4796 (per-word) and 4822-4877
//! (whole-string) modifier dispatch. The base set (`:t :h :r :e :s`)
//! is covered in glob_colon_modifiers.rs; this file pins the
//! additional ports so future refactors don't silently revert them.

use zsh::glob::apply_colon_modifiers;

#[test]
fn lower_uppercase() {
    assert_eq!(apply_colon_modifiers("Cargo.TOML", ":l"), "cargo.toml");
    assert_eq!(apply_colon_modifiers("hello.txt", ":u"), "HELLO.TXT");
    // Chained — uppercase then take basename.
    assert_eq!(apply_colon_modifiers("/tmp/ABC/foo.RS", ":u:t"), "FOO.RS");
    // Chained — basename then lowercase.
    assert_eq!(apply_colon_modifiers("/tmp/Foo/BAR.RS", ":t:l"), "bar.rs");
}

#[test]
fn quote_metachars() {
    // Backslash-escapes shell metacharacters per zsh's QT_BACKSLASH.
    assert_eq!(
        apply_colon_modifiers("foo bar baz", ":q"),
        "foo\\ bar\\ baz"
    );
    assert_eq!(apply_colon_modifiers("a;b|c&d", ":q"), "a\\;b\\|c\\&d");
    assert_eq!(
        apply_colon_modifiers("hi$world`cmd`", ":q"),
        "hi\\$world\\`cmd\\`"
    );
    // No metachars — passthrough.
    assert_eq!(apply_colon_modifiers("plain.txt", ":q"), "plain.txt");
}

#[test]
fn unquote_strips_quoting() {
    // Backslash-escapes:
    assert_eq!(apply_colon_modifiers("foo\\ bar", ":Q"), "foo bar");
    // Single-quoted run — literal contents.
    assert_eq!(apply_colon_modifiers("'hello world'", ":Q"), "hello world");
    // Double-quoted with backslash-escapes (\" \\ \$ \` only).
    assert_eq!(apply_colon_modifiers("\"a\\\"b\\$c\"", ":Q"), "a\"b$c");
    // q then Q is a round-trip for non-quote metachars.
    assert_eq!(apply_colon_modifiers("foo bar;baz", ":q:Q"), "foo bar;baz");
}

#[test]
fn abs_lexically_normalizes() {
    // `:a` collapses `.` and `..` without filesystem access.
    let cwd = std::env::current_dir().unwrap();
    let cwd_str = cwd.to_string_lossy().into_owned();

    assert_eq!(
        apply_colon_modifiers("/usr/lib/../bin/./ls", ":a"),
        "/usr/bin/ls"
    );

    // Relative path gets prepended with cwd.
    let got = apply_colon_modifiers("foo/bar", ":a");
    assert!(
        got.starts_with(&cwd_str),
        "expected cwd prefix on relative `:a`; got {got}"
    );
    assert!(got.ends_with("/foo/bar"), "got {got}");

    // Absolute path stays absolute, just normalized.
    assert_eq!(apply_colon_modifiers("/a/b/../c", ":a"), "/a/c");
}

#[test]
fn realpath_resolves_when_possible() {
    // `:A` and `:P` both canonicalize. Use a tempdir + symlink so the
    // resolution is observable. On miss, falls back to `:a` lexical.
    let tmp = tempfile::TempDir::new().unwrap();
    let real = tmp.path().join("real.txt");
    std::fs::write(&real, b"").unwrap();

    let real_canon = real.canonicalize().unwrap().to_string_lossy().into_owned();
    let real_str = real.to_string_lossy().into_owned();

    assert_eq!(apply_colon_modifiers(&real_str, ":A"), real_canon);
    assert_eq!(apply_colon_modifiers(&real_str, ":P"), real_canon);

    // Non-existent — falls back to lexical absolute.
    let missing = tmp.path().join("does/not/exist");
    let got = apply_colon_modifiers(&missing.to_string_lossy(), ":A");
    assert!(got.starts_with('/'), "expected absolute path, got {got}");
}

#[test]
fn command_resolves_in_path() {
    // `:c` looks up bare command name in $PATH. Use `sh` which is
    // guaranteed to exist on macOS+Linux.
    let got = apply_colon_modifiers("sh", ":c");
    assert!(
        got.ends_with("/sh") && got.starts_with('/'),
        "expected /.../sh, got {got}"
    );

    // Containing a `/` is treated as already-a-path and returned
    // unchanged.
    assert_eq!(
        apply_colon_modifiers("./local/binary", ":c"),
        "./local/binary"
    );

    // Unknown command — returned unchanged.
    let unknown = "this_command_does_not_exist_qwertyuiop";
    assert_eq!(apply_colon_modifiers(unknown, ":c"), unknown);
}

#[test]
fn capital_s_is_alias_for_s() {
    // `:S` and `:s` go through the same substitution branch in
    // subst.c:4764-4770. Verify alias.
    assert_eq!(
        apply_colon_modifiers("hello world", ":S/world/Earth/"),
        "hello Earth"
    );
    assert_eq!(apply_colon_modifiers("a b a c", ":gS/a/Z/"), "Z b Z c");
}

#[test]
fn long_chain_pipelines() {
    // `:r:t:u` — strip extension, take basename, uppercase.
    assert_eq!(
        apply_colon_modifiers("/path/to/Cargo.toml", ":r:t:u"),
        "CARGO"
    );
    // Subst then case-fold.
    assert_eq!(
        apply_colon_modifiers("foo.bar.baz", ":s/bar/QUX/:l"),
        "foo.qux.baz"
    );
    // Quote then unquote — round trip on a path with spaces.
    assert_eq!(
        apply_colon_modifiers("/tmp/some path/file.txt", ":q:Q"),
        "/tmp/some path/file.txt"
    );
}

#[test]
fn unknown_modifier_stops_chain_cleanly() {
    // `:Z` is unsupported — chain stops, prior result preserved.
    assert_eq!(apply_colon_modifiers("Cargo.toml", ":r:Z:t"), "Cargo");
}
