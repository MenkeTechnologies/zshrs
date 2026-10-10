//! One csh word → the zsh word with the same meaning.
//!
//! Reference: tcsh 6.21 (`/bin/csh` on macOS). Every rule below was checked
//! by running the csh word under `tcsh -f` and the translation under
//! `zsh -f`, comparing the argument vectors.
//!
//! # Variable model
//!
//! * csh `set` variables are always lists, so they are zsh **arrays**:
//!   the command layer must emit `x=(a b c)` for `set x = (a b c)` and
//!   `x=(v)` for `set x = v`. `$#x` is `${#x}` and `$x[2]` is `${x[2]}`
//!   only under that convention.
//! * `setenv` variables are zsh scalars (`export E=v`). csh and zsh keep
//!   one namespace in this translation, so a `set` and a `setenv` of the
//!   same name share one slot (csh keeps two; the shell variable wins).
//! * The csh specials `home user term shell cwd owd uid euid gid` map to the
//!   zsh parameters of [`zsh_var_name`]; `$status` is `$?`; `$*` is `$argv`.
//!
//! # Expansion rules that differ from zsh (all verified against tcsh)
//!
//! * An unquoted `$x` is split on whitespace *per element* and empty words
//!   vanish; `"$x"` joins the elements with one space. zsh needs `${=x}` for
//!   the first and `"${x}"` for the second.
//! * `\` inside `"…"` is a literal backslash (`"a\$b"` is `a\` + `$b`
//!   expanded; `"a\"` ends the string). zsh treats `\$ \" \\ \`` as escapes,
//!   so every csh `\` inside `"…"` becomes `\\`.
//! * Inside `'…'` nothing is special, not even `\'`; same as zsh.
//! * `{a}` loses its braces (`a`), `{a,b}` stays, `{1..3}` is the literal
//!   `1..3`, `{}` is kept only as a whole word (`find -exec {} \;`), and
//!   `x{}y` is `xy`.
//! * `$x:h` etc. without `g` modify the first word only — unless the first
//!   word is changed by two or more chained modifiers, in which case tcsh
//!   modifies every word (translated as: two or more modifiers means every
//!   word; `$q:s/a/X/:h` where `:h` is a no-op on word 1 is the exception
//!   tcsh treats as first-word-only). `:h`
//!   is `${x%/*}` and `:t` is `${x##*/}` (zsh `:h`/`:t` disagree with csh
//!   on `abc`, `a/`, `/abc`); `:r` and `:e` equal the zsh modifiers.
//! * `:u`/`:l` act on the first lowercase/uppercase *letter* of a word.
//! * Backquote output is split into words at whitespace; inside `"…"` it is
//!   split at newlines only, one word per non-empty line (`"x`cmd`y"` glues
//!   `x` to the first line and `y` to the last).
//!
//! # Checked translation
//!
//! [`translate_word_checked`] is [`translate_word`] plus the run-time
//! behaviour of tcsh that has no zsh spelling at the word level without a
//! guard. Each guard is a `${…}` that expands to nothing unless tcsh would
//! have failed. A failing guard aborts with status 1 and the tcsh text as
//! the end of the stderr line (zsh puts `<script>:<line>: __csh: ` or, for a
//! variable, `<script>:<line>: ` in front); a non-interactive shell exits,
//! an interactive one returns to the prompt.
//!
//! * `x: Undefined variable.` for `$x`, `${x}`, `$x[n]`, `$#x`, `$%x`,
//!   `$x:h`, inside `"…"` and for a variable used as a subscript bound.
//!   `$?x`, `$1`…`$9` (empty when absent) and the variables tcsh itself
//!   defines (`argv status cwd owd user shell uid euid gid tcsh version
//!   tty shlvl group HOST HOSTTYPE OSTYPE MACHTYPE VENDOR`) are exempt.
//!   `$home`/`$term` are tested through `HOME`/`TERM` and reported under
//!   the csh name.
//! * `x: Subscript out of range.`: the bounds of `$x[n]`/`$x[n-m]` are
//!   `lo`..`hi` (`n-` ends at the size, `-m` starts at 1). tcsh aborts when
//!   `hi` is past the size or when `lo` is 0 and `hi` is not; `lo > hi`,
//!   `$x[0]`, `$x[0-0]`, `$x[n-]` past the end and `$x[*]` are empty.
//!   `$1[2]` and `$*[2]` take no subscript in tcsh (the `[2]` is glob
//!   text) and are translated that way in both entry points.
//! * `Unknown user: u.` for a word starting `~u`. zsh aborts on its own
//!   (`no such user or named directory`) before any guard in the word can
//!   run, so the lookup is a subshell probe and the directory a quoted
//!   `$(print -r -- ~u)`.
//! * `Missing '}'.` for an unquoted `$v` (or `$v:q`) that is more than one
//!   word inside `{…}`.
//! * An unquoted variable value is globbed. tcsh passes the words of the
//!   value to the glob stage: `set x = '*.c'; echo $x` lists files, a
//!   leading `~` expands, `[ab]` and `?` match. `( ) | < > & ^ #` stay
//!   literal. The words are left untouched unless one of them has a glob
//!   character (`* ? [ {`) or a leading `~`; only then is the whole list
//!   passed through `${~…}` with `( ) | < > ^ #` and a non-leading `~`
//!   backslash-quoted. `"$x"`, `$x:q` and `$x:x` are never globbed.
//! * A whole-word `"`cmd`"` yields no word for empty or newline-only
//!   output (zsh would keep one empty word).
//! * `$#E` on a `setenv` variable is the value of `$E` (tcsh has no count
//!   for an environment variable); on a name that holds an array at run
//!   time it is the element count. `$#home`, `$#cwd` and the other mapped
//!   specials are 1, like any one-word csh list.
//!
//! The checked output needs `setopt cshnullglob extendedglob` (the driver
//! preamble already sets both; `extendedglob` is used by the `(#m)`/`(#b)`
//! flags of the escaping and of `:gu`/`:gl`).
//!
//! # Not representable (the translation is best effort, noted for callers)
//!
//! * **Error scope**: `Unknown user`, `Missing '}'.` and `No match.`
//!   abort only the command in tcsh when it is an external program (the
//!   script goes on with status 1); zsh ends the script for all three.
//!   For a builtin both shells end the script.
//! * **Backquote bodies** are translated by [`super::cmds`], which calls
//!   [`translate_word`]: `` `echo $nosuch` `` is not guarded and the output
//!   of an unquoted backquote is not globbed (tcsh globs it).
//! * **Brace expansion of values**: tcsh brace-expands the words of a
//!   value (`set x = '{a,b}.c'; echo $x` is `a.c b.c`); zsh keeps the
//!   braces.
//! * **A mixed list** (`set x = ('*.c' '(par)')`) is globbed as a whole;
//!   the elements without glob characters but with `( ) | < > ^ #` keep
//!   the quoting backslashes.
//! * **`$E[n]`** on a `setenv` variable is the value followed by the glob
//!   text `[n]` in tcsh; an upper-case name is still subscripted here
//!   because a csh array may have an upper-case name too.
//! * **Two adjacent quoted backquotes** (`"`a``b`"`) are not merged into
//!   one word list.
//! * **`no match`** text: tcsh prints `cmd: No match.` and errors only when
//!   *every* glob word of a command matched nothing, silently dropping the
//!   unmatched ones otherwise. That is zsh `setopt cshnullglob`; the driver
//!   preamble must set it. zsh's message is `no match`.
//! * **History**: `!` in a csh script triggers history substitution
//!   (`a!b` → `b: Event not found.`). zsh scripts do not; `\!` and `"\!"`
//!   translate to a plain `!`.
//! * **Modifier `:s`** operands are literal strings (no `&`, no `\` escape in
//!   tcsh). Operands holding `/ \ } $ ` " '` or blanks have no safe zsh
//!   spelling here and the modifier is dropped.
//! * **`:gu` / `:gl`** (and `:u`/`:l` chained with another modifier) rewrite
//!   every element with a `(#b)` pattern; that needs `setopt extendedglob`
//!   in the driver preamble.
//! * Nested backquotes, `$x:` followed by a non-modifier (tcsh: `Bad :
//!   modifier in $`), `$x[$list]` with a multi-word bound (tcsh: `Missing
//!   '-'.`) and other tcsh syntax errors are passed through leniently
//!   instead of being diagnosed.

use super::cmds;

/// zsh parameter name for a csh variable name.
///
/// `home→HOME`, `user→USER`, `term→TERM`, `shell→SHELL`, `cwd→PWD`,
/// `owd→OLDPWD`, `uid→UID`, `euid→EUID`, `gid→GID`; `*` (as in `$*`) is
/// `argv`. Everything else (`path`, `cdpath`, `argv`, user names) is the
/// same name in both shells. Assignment translation must use this too so a
/// `set home = …` and a later `$home` agree.
pub fn zsh_var_name(name: &str) -> &str {
    match name {
        "home" => "HOME",
        "user" => "USER",
        "term" => "TERM",
        "shell" => "SHELL",
        "cwd" => "PWD",
        "owd" => "OLDPWD",
        "uid" => "UID",
        "euid" => "EUID",
        "gid" => "GID",
        "*" => "argv",
        other => other,
    }
}

/// Translate a single csh word, quotes and all (`"$x[2]"`, `$#argv`,
/// `${?v}`, `$x:h`, `~user`, …). Input is one word from
/// [`super::lex::split_words`].
pub fn translate_word(word: &str) -> String {
    translate(word, false)
}

/// [`translate_word`] plus the tcsh run-time behaviour a plain
/// translation cannot have (see "Checked translation" at the top of this
/// file): the word aborts the command like tcsh for an undefined variable,
/// an out-of-range subscript, an unknown `~user` or a multi-word expansion
/// inside `{…}`, and unquoted variable values are globbed.
///
/// The result is longer and uses `extendedglob` pattern syntax, so it is a
/// separate entry point; callers that need only the argument vector (type
/// sniffing, `plain_param`-style rewrites) keep calling [`translate_word`].
pub fn translate_word_checked(word: &str) -> String {
    translate(word, true)
}

fn translate(word: &str, checked: bool) -> String {
    let chars: Vec<char> = word.chars().collect();
    if chars == ['{', '}'] {
        return "{}".to_string();
    }
    let mut out = String::new();
    let cx = Cx {
        checked,
        in_brace: false,
    };
    if checked {
        if let Some(lines) = whole_word_backquote(&chars) {
            return lines;
        }
    }
    bare(&chars, &mut out, true, cx);
    out
}

/// `"`cmd`"` as a whole word. tcsh yields one word per non-empty output
/// line and no word at all for empty or newline-only output, where a
/// quoted zsh array expansion would leave one empty word. The expansion is
/// therefore left unquoted (array elements are not re-split or globbed).
fn whole_word_backquote(c: &[char]) -> Option<String> {
    let inner = c.strip_prefix(&['"'])?.strip_suffix(&['"'])?;
    let body = inner.strip_prefix(&['`'])?.strip_suffix(&['`'])?;
    if body.contains(&'`') || body.contains(&'"') {
        return None;
    }
    let src: String = body.iter().collect();
    let zsh = cmds::translate_line(&src).unwrap_or(src);
    Some(format!("${{(@)${{(@f)\"$({})\"}}:#}}", zsh.trim()))
}

/// Where a `$…` expansion sits: in an unquoted word or inside `"…"`.
#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Bare,
    Quoted,
}

/// Translation settings that travel down the recursive-descent helpers.
#[derive(Clone, Copy)]
struct Cx {
    /// emit the tcsh error guards and glob unquoted values
    checked: bool,
    /// inside a `{…}` alternative, where tcsh rejects a multi-word `$x`
    in_brace: bool,
}

/// Translate unquoted csh text. `word_start` is true when `c` begins a
/// word (or a brace alternative at the start of a word), which is where
/// `~` expands and a leading `=` must be protected from zsh's `=cmd`.
fn bare(c: &[char], out: &mut String, word_start: bool, cx: Cx) {
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        match ch {
            '\\' => match c.get(i + 1) {
                // backslash-newline is a word break in csh; nothing to keep
                Some('\n') => i += 2,
                Some(&next) => {
                    out.push('\\');
                    out.push(next);
                    i += 2;
                }
                None => {
                    out.push_str("\\\\");
                    i += 1;
                }
            },
            '\'' => match c[i + 1..].iter().position(|&x| x == '\'') {
                Some(n) => {
                    out.extend(&c[i..=i + 1 + n]);
                    i += n + 2;
                }
                None => {
                    out.push('\'');
                    out.extend(&c[i + 1..]);
                    out.push('\'');
                    i = c.len();
                }
            },
            '"' => i = dquoted(c, i + 1, out, cx),
            '`' => i = backquote(c, i, Ctx::Bare, out),
            '$' => i = dollar(c, i + 1, Ctx::Bare, cx, out),
            '{' => i = brace(c, i, word_start && i == 0, cx, out),
            '~' if word_start && i == 0 => {
                let user: String = c[1..].iter().take_while(|&&x| is_user_char(x)).collect();
                if cx.checked && !user.is_empty() {
                    out.push_str(&known_user(&user));
                    i += 1 + user.chars().count();
                } else {
                    out.push('~');
                    i += 1;
                }
            }
            '~' | '=' | '}' => {
                // `=cmd`, mid-word `~` and a stray `}` are inert in csh but
                // active (or reserved) in zsh.
                let literal = ch != '=' || (word_start && i == 0);
                if literal {
                    out.push('\\');
                }
                out.push(ch);
                i += 1;
            }
            _ => {
                out.push(ch);
                i += 1;
            }
        }
    }
}

/// Translate the body of `"…"` starting after the opening quote; returns
/// the index after the closing quote (or the end of input when unclosed).
fn dquoted(c: &[char], mut i: usize, out: &mut String, cx: Cx) -> usize {
    out.push('"');
    while i < c.len() {
        match c[i] {
            '"' => {
                out.push('"');
                return i + 1;
            }
            '\\' => match c.get(i + 1) {
                // backslash-newline keeps the newline, drops the backslash
                Some('\n') => {
                    out.push('\n');
                    i += 2;
                }
                // history escape: "\!" is "!"
                Some('!') => {
                    out.push('!');
                    i += 2;
                }
                // otherwise a literal backslash; what follows is re-read
                _ => {
                    out.push_str("\\\\");
                    i += 1;
                }
            },
            '$' => i = dollar(c, i + 1, Ctx::Quoted, cx, out),
            '`' => i = backquote(c, i, Ctx::Quoted, out),
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out.push('"');
    c.len()
}

/// `` `cmd` `` at `c[i]`. Returns the index after the closing backquote.
/// The body is csh source: it is translated as a command line.
fn backquote(c: &[char], i: usize, ctx: Ctx, out: &mut String) -> usize {
    let Some(n) = c[i + 1..].iter().position(|&x| x == '`') else {
        out.push_str("\\`");
        return i + 1;
    };
    let body: String = c[i + 1..i + 1 + n].iter().collect();
    let zsh = cmds::translate_line(&body).unwrap_or(body);
    let zsh = zsh.trim();
    match ctx {
        Ctx::Bare => out.push_str(&format!("$({zsh})")),
        // csh joins the output lines with single spaces inside "…"
        Ctx::Quoted => out.push_str(&format!("${{(@)${{(@f)\"$({zsh})\"}}:#}}")),
    }
    i + n + 2
}

/// Index of the `}` matching the `{` at `c[open]`, skipping quotes,
/// backslash escapes and nested `${…}`/`{…}`.
fn find_brace_end(c: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = open;
    while i < c.len() {
        match c[i] {
            '\\' => i += 1,
            q @ ('\'' | '"' | '`') => i += c[i + 1..].iter().position(|&x| x == q)? + 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split brace contents at top-level commas.
fn split_alternatives(c: &[char]) -> Vec<&[char]> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' => i += 1,
            q @ ('\'' | '"' | '`') => {
                if let Some(n) = c[i + 1..].iter().position(|&x| x == q) {
                    i += n + 1;
                }
            }
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&c[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&c[start..]);
    parts
}

/// Brace group at `c[open]`. csh drops the braces of a group without a
/// comma, drops an empty `{}` inside a longer word, and errors on an
/// unclosed `{` (kept here as a literal).
fn brace(c: &[char], open: usize, word_start: bool, cx: Cx, out: &mut String) -> usize {
    let Some(end) = find_brace_end(c, open) else {
        out.push_str("\\{");
        return open + 1;
    };
    let alternatives = split_alternatives(&c[open + 1..end]);
    let inner = Cx { in_brace: true, ..cx };
    if alternatives.len() > 1 {
        out.push('{');
        for (n, alt) in alternatives.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            bare(alt, out, word_start, inner);
        }
        out.push('}');
    } else {
        bare(alternatives[0], out, word_start, inner);
    }
    end + 1
}

/// What a `$` reference asks for.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// `$x`, `$x[2]`, `$x:h`
    Value,
    /// `$#x`
    Count,
    /// `$?x`
    IsSet,
    /// `$%x`: characters, not words
    Length,
}

/// A `$x[i-j]` subscript, already spelled for zsh.
enum Sub {
    All,
    One(String),
    Range(String, String),
}

/// One `:h`-style modifier.
struct Modifier {
    letter: char,
    /// `:as/l/r/`: replace every occurrence in a word
    all_occurrences: bool,
    /// `:s/l/r/` operands, already escaped for `${x/l/r}`
    subst: Option<(String, String)>,
}

struct Reference {
    kind: Kind,
    /// csh name: identifier, digits, `*`, or empty for `$#` / `$?` alone
    name: String,
    sub: Option<Sub>,
    modifiers: Vec<Modifier>,
    /// `g` seen in the modifier chain: every word is modified
    global: bool,
    /// `:q` seen: words are not re-split
    quote: bool,
    /// `:q` or `:x` seen: the words are not globbed
    no_glob: bool,
    /// csh names of the variables read by subscript bounds (`$x[$i-$#y]`)
    bound_names: Vec<String>,
}

/// `$…` with `i` just past the `$`. Returns the index after the reference.
fn dollar(c: &[char], i: usize, ctx: Ctx, cx: Cx, out: &mut String) -> usize {
    match c.get(i) {
        None => {
            out.push_str("\\$");
            i
        }
        Some('$') => {
            out.push_str("$$");
            i + 1
        }
        Some('<') => {
            // csh reads one raw line; unquoted the result is split/globbed
            // exactly like command substitution output.
            out.push_str("$(IFS= read -r; print -r -- $REPLY)");
            i + 1
        }
        Some('{') => {
            let end = c[i..].iter().position(|&x| x == '}').map(|n| i + n);
            let parsed = end.and_then(|e| parse_reference(&c[i + 1..e]).map(|(r, used)| (r, used, e)));
            match parsed {
                Some((r, used, e)) if used == e - i - 1 => {
                    render(&r, ctx, cx, out);
                    e + 1
                }
                _ => {
                    out.push_str("\\$");
                    i
                }
            }
        }
        Some(_) => match parse_reference(&c[i..]) {
            Some((r, used)) => {
                render(&r, ctx, cx, out);
                i + used
            }
            None => {
                out.push_str("\\$");
                i
            }
        },
    }
}

/// Parse a reference (text after `$` or inside `${…}`); returns it with the
/// number of chars consumed.
fn parse_reference(c: &[char]) -> Option<(Reference, usize)> {
    let mut p = 0;
    let kind = match c.first()? {
        '#' => Kind::Count,
        '?' => Kind::IsSet,
        '%' => Kind::Length,
        _ => Kind::Value,
    };
    if kind != Kind::Value {
        p = 1;
    }
    let (name, np) = parse_name(c, p);
    let mut r = Reference {
        kind,
        name,
        sub: None,
        modifiers: Vec::new(),
        global: false,
        quote: false,
        no_glob: false,
        bound_names: Vec::new(),
    };
    if r.name.is_empty() {
        // bare `$#` (= $#argv) and `$?` (= $status) are the only nameless forms
        return (kind == Kind::Count || kind == Kind::IsSet).then_some((r, p));
    }
    if r.name == "*" && matches!(kind, Kind::Count | Kind::IsSet) {
        return None; // tcsh: `* not allowed with $# or $?.`
    }
    if kind == Kind::Count && r.name.chars().all(|ch| ch.is_ascii_digit()) {
        return None; // tcsh: `$#<num> is not allowed.`
    }
    p = np;
    // tcsh takes no subscript on `$1` or `$*`: `$1[2]` is the value of `$1`
    // followed by the glob text `[2]`.
    let positional = r.name == "*" || r.name.chars().all(|ch| ch.is_ascii_digit());
    if matches!(kind, Kind::Value | Kind::Length) && !positional && c.get(p) == Some(&'[') {
        let mut names = Vec::new();
        if let Some((sub, used)) = parse_subscript(&c[p..], &mut names) {
            r.sub = Some(sub);
            r.bound_names = names;
            p += used;
        }
    }
    if kind == Kind::Value {
        p += parse_modifiers(&c[p..], &mut r);
    }
    Some((r, p))
}

/// Identifier, digit string or `*` at `c[p..]`; empty when none.
fn parse_name(c: &[char], p: usize) -> (String, usize) {
    let first = match c.get(p) {
        Some(&ch) => ch,
        None => return (String::new(), p),
    };
    let accept: fn(char) -> bool = if first == '*' {
        |_| false
    } else if first.is_ascii_digit() {
        |ch| ch.is_ascii_digit()
    } else if first.is_ascii_alphabetic() || first == '_' {
        |ch| ch.is_ascii_alphanumeric() || ch == '_'
    } else {
        return (String::new(), p);
    };
    let mut end = p + 1;
    while c.get(end).is_some_and(|&ch| accept(ch)) {
        end += 1;
    }
    (c[p..end].iter().collect(), end)
}

/// `[*]`, `[n]`, `[n-m]`, `[n-]`, `[-m]`, `[-]` where each bound is digits
/// or a `$` reference. Returns the subscript and the chars consumed
/// (brackets included).
fn parse_subscript(c: &[char], bound_names: &mut Vec<String>) -> Option<(Sub, usize)> {
    let close = c.iter().position(|&x| x == ']')?;
    let inner = &c[1..close];
    if inner == ['*'] {
        return Some((Sub::All, close + 1));
    }
    if inner.is_empty() {
        return None;
    }
    // csh substitutes first and splits on `-` afterwards: `$x[$n-2]` is the
    // range `$n`..`2`, not arithmetic. Bounds therefore never contain `-`.
    let mut bounds: Vec<String> = Vec::new();
    let mut cur: Vec<char> = Vec::new();
    let mut dashes = 0;
    for &ch in inner {
        if ch == '-' {
            bounds.push(bound(&cur, bound_names)?);
            cur.clear();
            dashes += 1;
        } else {
            cur.push(ch);
        }
    }
    bounds.push(bound(&cur, bound_names)?);
    let sub = match (dashes, bounds.as_slice()) {
        (0, [n]) if !n.is_empty() => Sub::One(n.clone()),
        (1, [lo, hi]) => match (lo.is_empty(), hi.is_empty()) {
            (true, true) => Sub::All,
            (true, false) => Sub::Range("1".into(), hi.clone()),
            (false, true) => Sub::Range(lo.clone(), "-1".into()),
            (false, false) => Sub::Range(lo.clone(), hi.clone()),
        },
        _ => return None,
    };
    Some((sub, close + 1))
}

/// A subscript bound: empty, digits, `$name`, `${name}` or `$#name`,
/// spelled for use inside a zsh subscript.
fn bound(c: &[char], bound_names: &mut Vec<String>) -> Option<String> {
    if c.iter().all(|ch| ch.is_ascii_digit()) {
        return Some(c.iter().collect());
    }
    let after = c.strip_prefix(&['$'])?;
    let braced = after.first() == Some(&'{') && after.last() == Some(&'}');
    let text = if braced { &after[1..after.len() - 1] } else { after };
    let (r, used) = parse_reference(text)?;
    (used == text.len() && matches!(r.kind, Kind::Value | Kind::Count) && r.sub.is_none())
        .then(|| {
            bound_names.push(r.name.clone());
            let name = zsh_var_name(&r.name);
            match r.kind {
                Kind::Count => format!("${{#{name}}}"),
                _ => format!("${{{name}}}"),
            }
        })
}

/// Parse `:[ga]?[htreulqxs]` modifiers, appending to `r`; returns the
/// chars consumed. Stops at the first `:` that is not a modifier.
fn parse_modifiers(c: &[char], r: &mut Reference) -> usize {
    let mut p = 0;
    while c.get(p) == Some(&':') {
        let mut q = p + 1;
        let (mut g, mut a) = (false, false);
        while let Some(&f) = c.get(q) {
            match f {
                'g' => g = true,
                'a' => a = true,
                _ => break,
            }
            q += 1;
        }
        let Some(&letter) = c.get(q) else { break };
        match letter {
            'h' | 't' | 'r' | 'e' | 'u' | 'l' => r.modifiers.push(Modifier {
                letter,
                all_occurrences: false,
                subst: None,
            }),
            'q' | 'x' => {
                r.quote |= letter == 'q';
                r.no_glob = true;
            }
            's' => {
                let Some((subst, used)) = parse_substitution(&c[q + 1..]) else { break };
                r.modifiers.push(Modifier {
                    letter,
                    all_occurrences: a,
                    subst,
                });
                q += used;
            }
            _ => break,
        }
        r.global |= g;
        p = q + 1;
    }
    p
}

/// Operands of `:s<d>l<d>r<d>` (text after the `s`). The trailing
/// delimiter is optional. Returns the zsh-escaped operands (`None` when
/// they cannot be spelled safely) and the chars consumed.
fn parse_substitution(c: &[char]) -> Option<(Option<(String, String)>, usize)> {
    let delim = *c.first()?;
    if delim.is_alphanumeric() || delim == ':' {
        return None;
    }
    let l_end = 1 + c[1..].iter().position(|&x| x == delim)?;
    let rest = &c[l_end + 1..];
    let r_len = rest
        .iter()
        .position(|&x| x == delim || x.is_whitespace() || x == '"')
        .unwrap_or(rest.len());
    let closed = rest.get(r_len) == Some(&delim);
    let used = l_end + 1 + r_len + usize::from(closed);
    let lhs: String = c[1..l_end].iter().collect();
    let rhs: String = rest[..r_len].iter().collect();
    let unsafe_char = |ch: char| ch.is_whitespace() || "/\\}$`\"'".contains(ch);
    if lhs.is_empty() || lhs.chars().any(unsafe_char) || rhs.chars().any(unsafe_char) {
        return Some((None, used));
    }
    // l is a zsh pattern there but a plain string in csh: backslash every
    // character that is not a letter or digit.
    let mut pattern = String::new();
    for ch in lhs.chars() {
        if !ch.is_alphanumeric() && ch != '_' {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    Some((Some((pattern, rhs)), used))
}

/// Emit the zsh spelling of `r`. In checked mode the tcsh error guards
/// come first: each is a `${…}` that expands to nothing unless tcsh would
/// have aborted the command.
fn render(r: &Reference, ctx: Ctx, cx: Cx, out: &mut String) {
    if cx.checked {
        out.push_str(&guards(r, ctx, cx));
    }
    let name = zsh_var_name(if r.name == "status" { "?" } else { &r.name });
    match r.kind {
        Kind::Count => out.push_str(&count(r, name, ctx, cx)),
        Kind::IsSet => {
            if r.name.is_empty() {
                out.push_str("$?");
            } else if r.name == "status" {
                out.push('1');
            } else if r.name.chars().all(|ch| ch.is_ascii_digit()) {
                // tcsh answers 1 for every positional index, set or not
                out.push('1');
            } else if r.name == "prompt" {
                // tcsh defines `$prompt` only in an interactive shell; zsh
                // always has the tied PS1, so ask the shell instead.
                out.push_str("${${${options[interactive]:#off}:+1}:-0}");
            } else {
                out.push_str(&format!("${{+{name}}}"));
            }
        }
        Kind::Length => {
            let base = base_text(name, &r.sub);
            out.push_str(&format!("${{#${{(j::){base}}}}}"));
        }
        Kind::Value => render_value(r, name, ctx, cx, out),
    }
}

/// `$#x`. A csh shell variable is a list, so this is the element count.
/// The mapped specials (`$#home`, `$#cwd`) are one-word lists. A `setenv`
/// variable has no `#` form in tcsh: `$#E` is just the value of `$E`;
/// the name is not known to be an array or a scalar until run time, so
/// the test is `${(t)E}`.
fn count(r: &Reference, name: &str, ctx: Ctx, cx: Cx) -> String {
    if r.name.is_empty() {
        return "${#argv}".to_string();
    }
    if name != r.name || r.name == "status" {
        return "1".to_string();
    }
    if !r.name.chars().any(|ch| ch.is_ascii_uppercase()) {
        return format!("${{#{name}}}");
    }
    let kind = format!("${{(t){name}}}");
    let array = format!("${{${{(M){kind}:#array*}}:+${{#{name}}}}}");
    let value = match ctx {
        Ctx::Bare => split_words(name, cx.checked),
        Ctx::Quoted => format!("${{{name}}}"),
    };
    let scalar = format!("${{${{{kind}:#array*}}:+{value}}}");
    format!("{array}{scalar}")
}

/// `${=expr}`: `expr` split at whitespace, and in checked mode globbed like
/// tcsh globs the words of an unquoted `$x`.
fn split_words(expr: &str, glob: bool) -> String {
    let words = format!("${{={expr}}}");
    if glob {
        glob_wrap(&words)
    } else {
        words
    }
}

/// `name` with its zsh subscript (`x`, `x[2]`, `x[2,-1]`).
fn base_text(name: &str, sub: &Option<Sub>) -> String {
    match sub {
        None | Some(Sub::All) => name.to_string(),
        Some(Sub::One(n)) => format!("{name}[{n}]"),
        Some(Sub::Range(lo, hi)) => format!("{name}[{lo},{hi}]"),
    }
}

fn render_value(r: &Reference, name: &str, ctx: Ctx, cx: Cx, out: &mut String) {
    let split = ctx == Ctx::Bare && !r.quote;
    let scalar_special = name == "?";
    let glob = cx.checked && !r.no_glob;
    let base = base_text(name, &r.sub);
    let wrap = |expr: &str| {
        if split {
            split_words(expr, glob)
        } else {
            expr.to_string()
        }
    };
    if r.modifiers.is_empty() {
        if scalar_special {
            out.push_str("$?");
        } else if split {
            out.push_str(&split_words(&base, glob));
        } else {
            out.push_str(&format!("${{{base}}}"));
        }
        return;
    }
    // tcsh modifies only the first word unless `g` or a chain of two
    // modifiers is present. A single string (`$x[2]`, a `setenv` scalar)
    // has only one word, so the distinction vanishes.
    let one_string = matches!(r.sub, Some(Sub::One(_))) || is_scalar_name(name);
    let every_word = r.global || r.modifiers.len() > 1 || one_string;
    if every_word {
        let mut expr = base.clone();
        for m in &r.modifiers {
            expr = apply(m, &expr, !one_string);
        }
        if ctx == Ctx::Quoted && !one_string {
            // csh joins the modified words with one space inside "…"
            expr = format!("${{(j: :){expr}}}");
        }
        out.push_str(&wrap(&expr));
        return;
    }
    // One modifier, first word only: modify element 1, pass the rest.
    let (first_el, rest_inner) = match &r.sub {
        Some(Sub::Range(..)) => {
            let elements = format!("${{(@){base}}}");
            (format!("${{{elements}[1]}}"), format!("{elements}[2,-1]"))
        }
        _ => (format!("${{{name}[1]}}"), format!("{name}[2,-1]")),
    };
    let first = apply(&r.modifiers[0], &first_el, false);
    let rest = format!("${{{rest_inner}}}");
    match ctx {
        Ctx::Bare => out.push_str(&format!("{} {}", wrap(&first), wrap(&rest))),
        Ctx::Quoted => out.push_str(&format!("{first}${{{rest_inner}:+ {rest}}}")),
    }
}

// ---- checked translation -------------------------------------------------

/// Abort the command with `msg` on stderr and status 1 (a non-interactive
/// zsh exits, an interactive one returns to the prompt like tcsh) by
/// expanding `${__csh?msg}` on a parameter that is never set. zsh prefixes
/// the text with `<script>:<line>: __csh: `; the tcsh text is the suffix.
/// Single quotes keep a `}` in `msg` from closing the expansion.
fn abort(msg: &str) -> String {
    format!("${{__csh?{msg}}}")
}

/// Expand to `then` when the zsh arithmetic expression `cond` is true and
/// to nothing otherwise. Only the taken branch is evaluated.
fn when(cond: &str, then: &str) -> String {
    format!("${{${{${{:-$(({cond}))}}#0}}:+{then}}}")
}

/// Variables tcsh defines itself that zsh lacks (or fills in later), plus
/// the positional forms, which are empty rather than undefined.
fn always_defined(name: &str) -> bool {
    const NAMES: [&str; 20] = [
        "argv", "status", "tcsh", "version", "tty", "shlvl", "owd", "cwd", "user", "shell", "uid",
        "euid", "gid", "group", "HOST", "HOSTTYPE", "OSTYPE", "MACHTYPE", "VENDOR", "*",
    ];
    NAMES.contains(&name) || name.chars().all(|ch| ch.is_ascii_digit())
}

/// `x: Undefined variable.` guard for a read of csh variable `name`.
/// A mapped special (`home` → `HOME`) is tested through its zsh name but
/// reported under the csh name.
fn undefined_guard(name: &str) -> String {
    if name.is_empty() || always_defined(name) {
        return String::new();
    }
    let msg = "Undefined variable.";
    let zname = zsh_var_name(name);
    if zname == name {
        format!("${{${{{name}?{msg}}}:+}}")
    } else {
        format!("${{${{{zname}-${{{name}?{msg}}}}}:+}}")
    }
}

/// `x: Subscript out of range.` guard. tcsh numbers the bounds `lo`..`hi`
/// (`$x[n]` is `n`..`n`, `$x[n-]` ends at the size, `$x[-m]` starts at 1)
/// and aborts when `hi` is past the size, or when `lo` is 0 and `hi` is
/// not. `lo > hi` is an empty result, not an error; `$x[0]` is empty.
fn range_guard(name: &str, sub: &Sub) -> String {
    let size = format!("${{#{}}}", zsh_var_name(name));
    let digits = |s: &str| !s.is_empty() && s.chars().all(|ch| ch.is_ascii_digit());
    let is_zero = |s: &str| digits(s) && s.chars().all(|ch| ch == '0');
    let nonzero = |s: &str| digits(s) && !is_zero(s);
    let cond = match sub {
        Sub::All => return String::new(),
        Sub::One(n) if is_zero(n) => return String::new(),
        Sub::One(n) => format!("{n} > {size}"),
        Sub::Range(lo, hi) if hi == "-1" => {
            if nonzero(lo) {
                return String::new();
            }
            if is_zero(lo) {
                format!("{size} != 0")
            } else {
                format!("{lo} == 0 && {size} != 0")
            }
        }
        Sub::Range(lo, hi) => {
            let over = format!("{hi} > {size}");
            if nonzero(lo) {
                over
            } else if is_zero(lo) {
                format!("{hi} != 0")
            } else {
                format!("{over} || ({lo} == 0 && {hi} != 0)")
            }
        }
    };
    when(&cond, &abort(&format!("{name}: Subscript out of range.")))
}

/// All guards for one `$` reference, in tcsh's check order: undefined
/// variables (the subject and any variable in a subscript bound), then the
/// subscript range, then the `{…}` word-count rule.
fn guards(r: &Reference, ctx: Ctx, cx: Cx) -> String {
    let mut g = String::new();
    if matches!(r.kind, Kind::Value | Kind::Count | Kind::Length) {
        g.push_str(&undefined_guard(&r.name));
    }
    for bound in &r.bound_names {
        g.push_str(&undefined_guard(bound));
    }
    if matches!(r.kind, Kind::Value | Kind::Length) {
        if let Some(sub) = &r.sub {
            g.push_str(&range_guard(&r.name, sub));
        }
    }
    // tcsh rebuilds the word around the expansion before it matches the
    // braces, so `{$v,z}` with two words in `$v` is `Missing '}'.`
    if cx.in_brace && ctx == Ctx::Bare && r.kind == Kind::Value && r.name != "status" {
        let base = base_text(zsh_var_name(&r.name), &r.sub);
        // `:q` keeps each element one word; otherwise the value is split
        let elements = if r.quote {
            format!("${{(@){base}}}")
        } else {
            format!("${{={base}}}")
        };
        let words = format!("${{#{elements}}}");
        g.push_str(&when(&format!("{words} > 1"), &abort("Missing '}'.")));
    }
    g
}

fn is_user_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

/// `~user` at the start of a word. zsh aborts on an unknown user with its
/// own text before any expansion in the word runs, so the lookup is made in
/// a subshell and reported with tcsh's `Unknown user: u.`; the directory
/// itself comes from a quoted command substitution (it may hold blanks).
fn known_user(user: &str) -> String {
    let probe = format!("$({{ : ~{user}; }} 2>/dev/null && print known)");
    let unknown = abort(&format!("Unknown user: {user}."));
    format!("${{${{{probe}:-{unknown}}}:+}}\"$(print -r -- ~{user})\"")
}

/// Words tcsh hands to the glob stage: any with `* ? [ {`, or a leading `~`.
const GLOB_WORD: &str = "(\\~*|*[*?\\[\\{]*)";

/// Glob the words of `words` (an array-valued `${=…}` expression) like
/// tcsh does for an unquoted variable. tcsh leaves a value with only
/// `( ) | < > ^ #` alone, so the words are left alone unless one of them
/// has a glob character; then every word is globbed with those characters
/// and any `~` past the first column backslash-quoted.
fn glob_wrap(words: &str) -> String {
    let count = format!("${{#${{(M){words}:#{GLOB_WORD}}}}}");
    let quoted = format!("${{(@){words}//(#m)[()|<>^#]/\\\\$MATCH}}");
    let escaped = format!("${{(@){quoted}//(#b)([^~])~/${{match[1]}}\\\\~}}");
    let globbed = format!("${{${{{count}:#0}}:+${{~{escaped}}}}}");
    let plain = format!("${{${{(M){count}:#0}}:+{words}}}");
    format!("{globbed}{plain}")
}

/// Names holding an upper-case letter are zsh scalars: `setenv` variables
/// and the mapped specials (`PWD`, `HOME`, …); so are positional
/// parameters. csh shell variables are lower-case lists (zsh arrays).
fn is_scalar_name(name: &str) -> bool {
    name == "?"
        || name.starts_with(|ch: char| ch.is_ascii_digit())
        || name.chars().any(|ch| ch.is_ascii_uppercase())
}

/// Apply one modifier to the zsh expression `e` (a complete `${…}`).
/// `array` says `e` may hold several elements: the operator then carries
/// the `(@)` flag so it is applied per element even inside `"…"`, where a
/// nested array would otherwise be joined first.
fn apply(m: &Modifier, e: &str, array: bool) -> String {
    let at = if array { "(@)" } else { "" };
    match m.letter {
        'h' => format!("${{{at}{e}%/*}}"),
        't' => format!("${{{at}{e}##*/}}"),
        'r' => format!("${{{at}{e}:r}}"),
        'e' => format!("${{{at}{e}:e}}"),
        's' => match &m.subst {
            Some((l, r)) if m.all_occurrences => format!("${{{at}{e}//{l}/{r}}}"),
            Some((l, r)) => format!("${{{at}{e}/{l}/{r}}}"),
            None => e.to_string(),
        },
        'u' => first_letter_case(e, array, 'a', 'z', 'U'),
        'l' => first_letter_case(e, array, 'A', 'Z', 'L'),
        _ => e.to_string(),
    }
}

/// `:u` / `:l`: change the case of the first letter of class `lo..=hi`
/// (`flag` is the zsh case flag, `U` or `L`). For one string the prefix
/// before that letter is peeled with plain `%%`/`#` operators (joined by
/// `${:-…}` so the result stays one expression); for a list a per-element
/// `(#b)` substitution is the only option and needs `setopt extendedglob`.
fn first_letter_case(e: &str, array: bool, lo: char, hi: char, flag: char) -> String {
    if array {
        format!("${{(@){e}/(#b)([^{lo}-{hi}]#)([{lo}-{hi}])/${{match[1]}}${{({flag})match[2]}}}}")
    } else {
        let prefix = format!("${{{e}%%[{lo}-{hi}]*}}");
        let tail = format!("${{{e}#{prefix}}}");
        format!("${{:-{prefix}${{({flag}){tail}[1]}}${{{tail}[2,-1]}}}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(w: &str) -> String {
        translate_word(w)
    }

    #[test]
    fn plain_text_and_globs_pass_through() {
        assert_eq!(t("foo/bar-1.c"), "foo/bar-1.c");
        assert_eq!(t("*.c"), "*.c");
        assert_eq!(t("[a-b]?.c"), "[a-b]?.c");
    }

    #[test]
    fn unquoted_variable_splits_per_element() {
        // tcsh: `set x = ("a b" c); printf '[%s]' $x` -> [a][b][c]
        assert_eq!(t("$x"), "${=x}");
        assert_eq!(t("${x}"), "${=x}");
        assert_eq!(t("pre$x.suf"), "pre${=x}.suf");
    }

    #[test]
    fn quoted_variable_joins_elements() {
        // tcsh: `echo "[$x]"` with x=("" b) -> [ b]
        assert_eq!(t("\"[$x]\""), "\"[${x}]\"");
        assert_eq!(t("\"$x[*]\""), "\"${x}\"");
    }

    #[test]
    fn variable_name_stops_at_non_identifier() {
        // tcsh: `$x-y` is $x then "-y"; `$x_y` is the variable x_y
        assert_eq!(t("$x-y"), "${=x}-y");
        assert_eq!(t("$x_y"), "${=x_y}");
    }

    #[test]
    fn braced_variable_ends_modifier_scope() {
        // tcsh: `echo ${x}:h` with x=a.b prints `a.b:h`
        assert_eq!(t("${x}:h"), "${=x}:h");
    }

    #[test]
    fn prompt_is_set_only_in_an_interactive_shell() {
        assert_eq!(t("$?prompt"), "${${${options[interactive]:#off}:+1}:-0}");
        assert_eq!(t("${?prompt}"), "${${${options[interactive]:#off}:+1}:-0}");
    }

    #[test]
    fn count_and_isset_forms() {
        assert_eq!(t("$#x"), "${#x}");
        assert_eq!(t("\"$#x\""), "\"${#x}\"");
        assert_eq!(t("${#x}"), "${#x}");
        assert_eq!(t("$#"), "${#argv}");
        assert_eq!(t("$?x"), "${+x}");
        assert_eq!(t("${?x}"), "${+x}");
        assert_eq!(t("$?"), "$?");
        assert_eq!(t("$?status"), "1");
        // tcsh: `$?N` is 1 for every positional index, argument or not
        assert_eq!(t("$?2"), "1");
        assert_eq!(t("$?5"), "1");
    }

    #[test]
    fn length_form_counts_characters_across_elements() {
        // tcsh: x=(aa bbb) -> $%x = 5, $%x[2] = 3
        assert_eq!(t("$%x"), "${#${(j::)x}}");
        assert_eq!(t("$%x[2]"), "${#${(j::)x[2]}}");
    }

    #[test]
    fn subscripts_are_ranges_not_arithmetic() {
        assert_eq!(t("$x[2]"), "${=x[2]}");
        assert_eq!(t("$x[2-3]"), "${=x[2,3]}");
        assert_eq!(t("$x[2-]"), "${=x[2,-1]}");
        // tcsh: $x[-2] is elements 1..2 and $x[-] is all of them
        assert_eq!(t("$x[-2]"), "${=x[1,2]}");
        assert_eq!(t("$x[-]"), "${=x}");
        assert_eq!(t("$x[*]"), "${=x}");
    }

    #[test]
    fn subscript_bounds_may_be_references() {
        // tcsh: x=(a b c d), y=3: $x[$y-4] is "c d", $x[$#x-1] is the
        // empty range 4..1 -- `-` separates, it does not subtract.
        assert_eq!(t("$x[$#x]"), "${=x[${#x}]}");
        assert_eq!(t("$x[$y-4]"), "${=x[${y},4]}");
        assert_eq!(t("$x[$#x-1]"), "${=x[${#x},1]}");
    }

    #[test]
    fn non_subscript_bracket_stays_a_glob() {
        // tcsh: `$x[1]]` is element 1 followed by a literal `]`
        assert_eq!(t("$x[1]]"), "${=x[1]}]");
        // `[a]` is not a subscript (tcsh errors); the bracket is kept
        assert_eq!(t("$x[a]"), "${=x}[a]");
    }

    #[test]
    fn positional_and_special_parameters() {
        assert_eq!(t("$1"), "${=1}");
        // tcsh: `$1abc` is $1 followed by abc; `$10` is the tenth argument
        assert_eq!(t("$1abc"), "${=1}abc");
        assert_eq!(t("$10"), "${=10}");
        assert_eq!(t("$argv[2]"), "${=argv[2]}");
        // a positional parameter is one string: no first-word split
        assert_eq!(t("$1:r"), "${=${1:r}}");
        assert_eq!(t("$*"), "${=argv}");
        assert_eq!(t("$$"), "$$");
        assert_eq!(t("$status"), "$?");
        assert_eq!(t("$cwd"), "${=PWD}");
        assert_eq!(t("$home"), "${=HOME}");
        assert_eq!(t("${?home}"), "${+HOME}");
    }

    #[test]
    fn invalid_dollar_forms_stay_literal() {
        // tcsh rejects these (`$#*`, `$#1`, `$?*`); `$ x` is literal
        assert_eq!(t("$#*"), "\\$#*");
        assert_eq!(t("$#1"), "\\$#1");
        assert_eq!(t("$"), "\\$");
        assert_eq!(t("a$"), "a\\$");
    }

    #[test]
    fn read_line_form() {
        // tcsh: `echo "[$<]"` keeps the line's inner spacing
        assert_eq!(t("$<"), "$(IFS= read -r; print -r -- $REPLY)");
        assert_eq!(t("\"$<\""), "\"$(IFS= read -r; print -r -- $REPLY)\"");
    }

    #[test]
    fn modifiers_on_one_element() {
        assert_eq!(t("$x[1]:h"), "${=${x[1]%/*}}");
        assert_eq!(t("${x[1]:t}"), "${=${x[1]##*/}}");
        assert_eq!(t("$x[2]:r"), "${=${x[2]:r}}");
        assert_eq!(t("$x[2]:e"), "${=${x[2]:e}}");
        // setenv scalars take the same path: `$PWD:h`
        assert_eq!(t("$cwd:t"), "${=${PWD##*/}}");
    }

    #[test]
    fn single_modifier_touches_first_word_only() {
        // tcsh: p=(/a/b.c /x/y.z abc): `$p:t` is "b.c /x/y.z abc" -- only
        // element 1 is modified, the rest is passed through as words.
        assert_eq!(t("$p:t"), "${=${${p[1]}##*/}} ${=${p[2,-1]}}");
        // inside "…" one word again; the separator appears only if there
        // is a second element
        assert_eq!(t("\"$p:h\""), "\"${${p[1]}%/*}${p[2,-1]:+ ${p[2,-1]}}\"");
        // a subscript range is "first of the range", not element 1 of p
        assert_eq!(
            t("$p[1-2]:r"),
            "${=${${${(@)p[1,2]}[1]}:r}} ${=${${(@)p[1,2]}[2,-1]}}"
        );
    }

    #[test]
    fn global_modifier_hits_every_word() {
        // tcsh: `$p:gh` shortens every word of the list
        assert_eq!(t("$p:gh"), "${=${(@)p%/*}}");
        assert_eq!(t("$p:gt"), "${=${(@)p##*/}}");
        assert_eq!(t("$p:gr"), "${=${(@)p:r}}");
        assert_eq!(t("${p:ge}"), "${=${(@)p:e}}");
        // inside "…" the words are modified one by one, then joined
        assert_eq!(t("\"$p:gt\""), "\"${(j: :)${(@)p##*/}}\"");
    }

    #[test]
    fn chained_modifiers_hit_every_word() {
        // tcsh: p=(/a/b.c /x/y.z /m/n.o): `$p:t:r` is "b y n"
        assert_eq!(t("$p:t:r"), "${=${(@)${(@)p##*/}:r}}");
    }

    #[test]
    fn quote_and_split_modifiers() {
        // tcsh: `:q` keeps each element one word (no split, no glob)
        assert_eq!(t("$x:q"), "${x}");
        assert_eq!(t("$x:x"), "${=x}");
        assert_eq!(t("\"$x:q\""), "\"${x}\"");
    }

    #[test]
    fn colon_after_variable_without_modifier_is_text() {
        // tcsh errors (`Bad : modifier`); `${host}:port` is the safe csh
        assert_eq!(t("\"$h:$p\""), "\"${h}:${p}\"");
        assert_eq!(t("$h:80"), "${=h}:80");
    }

    #[test]
    fn substitute_modifier() {
        // tcsh: p=(a.b.a c.a.d): `$p:s/a/X/` -> "X.b.a c.a.d" (first word,
        // first occurrence); `:as` every occurrence; `:gs` every word.
        assert_eq!(t("$p[1]:s/a/X/"), "${=${p[1]/a/X}}");
        assert_eq!(t("$p[1]:as/a/X/"), "${=${p[1]//a/X}}");
        assert_eq!(t("$p:gs/a/X/"), "${=${(@)p/a/X}}");
        // operands are literal strings in tcsh: `.` and `*` are not patterns
        assert_eq!(t("$p[1]:s/./_/"), "${=${p[1]/\\./_}}");
        assert_eq!(t("$p[1]:s,a,Z,"), "${=${p[1]/a/Z}}");
    }

    #[test]
    fn first_letter_case_modifiers() {
        // tcsh: w=(1abc Abc xyz): `$w[1]:u` is 1Abc (first lowercase
        // letter), `$w[2]:u` is ABc; `:l` mirrors it.
        assert_eq!(
            t("$w[1]:u"),
            "${=${:-${w[1]%%[a-z]*}${(U)${w[1]#${w[1]%%[a-z]*}}[1]}${${w[1]#${w[1]%%[a-z]*}}[2,-1]}}}"
        );
        assert_eq!(
            t("$w[2]:l"),
            "${=${:-${w[2]%%[A-Z]*}${(L)${w[2]#${w[2]%%[A-Z]*}}[1]}${${w[2]#${w[2]%%[A-Z]*}}[2,-1]}}}"
        );
        // lists need the per-element (#b) form (setopt extendedglob)
        assert_eq!(
            t("$w:gu"),
            "${=${(@)w/(#b)([^a-z]#)([a-z])/${match[1]}${(U)match[2]}}}"
        );
    }

    #[test]
    fn double_quote_backslash_is_literal() {
        // tcsh: "a\\b" prints a\\b ; "a\$HOME" prints a\ + $HOME's value
        assert_eq!(t("\"a\\\\b\""), "\"a\\\\\\\\b\"");
        assert_eq!(t("\"a\\$HOME\""), "\"a\\\\${HOME}\"");
        // `"a\"` is the string a\ -- the quote still closes it
        assert_eq!(t("\"a\\\""), "\"a\\\\\"");
        assert_eq!(t("\"\\!\""), "\"!\"");
    }

    #[test]
    fn single_quotes_block_everything() {
        assert_eq!(t("'$x `y` \\n {a,b}'"), "'$x `y` \\n {a,b}'");
        // tcsh: 'it\'s' is unterminated -- backslash does not escape in '…'
        assert_eq!(t("'a\\'b"), "'a\\'b");
    }

    #[test]
    fn mixed_quoting_in_one_word() {
        assert_eq!(t("a\"$x\"'$y'b"), "a\"${x}\"'$y'b");
    }

    #[test]
    fn unquoted_backslash_quotes_next_char() {
        // tcsh: `echo \$x \* \" \#x` -> $x * " #x
        assert_eq!(t("\\$x"), "\\$x");
        assert_eq!(t("\\*"), "\\*");
        assert_eq!(t("\\#x"), "\\#x");
        assert_eq!(t("\\{a,b\\}"), "\\{a,b\\}");
    }

    #[test]
    fn backquote_forms() {
        // tcsh: unquoted output splits into words; in "…" newlines become
        // single spaces and blank lines vanish.
        assert_eq!(t("`echo hi`"), "$(print -r -- hi)");
        assert_eq!(t("a`echo b`c"), "a$(print -r -- b)c");
        assert_eq!(t("\"`echo hi`\""), "\"${(@)${(@f)\"$(print -r -- hi)\"}:#}\"");
        assert_eq!(t("'`echo hi`'"), "'`echo hi`'");
        assert_eq!(t("\\`echo"), "\\`echo");
    }

    #[test]
    fn tilde_expands_only_at_word_start() {
        // tcsh: `echo ~root/x a:~/x x~ "~"` -> only the first expands
        assert_eq!(t("~"), "~");
        assert_eq!(t("~root/x"), "~root/x");
        assert_eq!(t("a~b"), "a\\~b");
        assert_eq!(t("\"~\""), "\"~\"");
        assert_eq!(t("'~'"), "'~'");
        assert_eq!(t("\\~"), "\\~");
    }

    #[test]
    fn leading_equals_is_not_command_expansion() {
        // tcsh: `echo =ls` prints =ls; zsh would expand =ls to /bin/ls
        assert_eq!(t("=ls"), "\\=ls");
        assert_eq!(t("a=b"), "a=b");
    }

    #[test]
    fn brace_alternatives_stay() {
        assert_eq!(t("{a,b}c"), "{a,b}c");
        assert_eq!(t("{a,{b,c}}"), "{a,{b,c}}");
        assert_eq!(t("{,a}"), "{,a}");
        assert_eq!(t("{*.c,zz}"), "{*.c,zz}");
        assert_eq!(t("{~,x}/a"), "{~,x}/a");
        assert_eq!(t("{a,b}$x"), "{a,b}${=x}");
    }

    #[test]
    fn brace_without_comma_loses_its_braces() {
        // tcsh: `echo {a} x{b}y {1..3} {}x x{}` -> a xby 1..3 x x
        assert_eq!(t("{a}"), "a");
        assert_eq!(t("x{b}y"), "xby");
        assert_eq!(t("{1..3}"), "1..3");
        assert_eq!(t("{}x"), "x");
        assert_eq!(t("x{}"), "x");
    }

    #[test]
    fn empty_braces_survive_only_as_a_whole_word() {
        // `find . -exec rm {} \;`
        assert_eq!(t("{}"), "{}");
    }

    #[test]
    fn quoted_braces_are_literal() {
        assert_eq!(t("\"{a,b}\""), "\"{a,b}\"");
        assert_eq!(t("'{a}'c"), "'{a}'c");
        assert_eq!(t("\"{a}\"{b}"), "\"{a}\"b");
    }

    #[test]
    fn unclosed_brace_is_literal() {
        // tcsh: `a{b` -> Missing }.  A lone `{` or `}` word is literal.
        assert_eq!(t("a{b"), "a\\{b");
        assert_eq!(t("{"), "\\{");
        assert_eq!(t("}"), "\\}");
    }

    #[test]
    fn braced_variable_inside_alternatives() {
        // tcsh: `echo {$HOME,b}` -> /Users/... b
        assert_eq!(t("{$HOME,b}"), "{${=HOME},b}");
    }

    #[test]
    fn trailing_unterminated_quote_is_closed() {
        assert_eq!(t("\"abc"), "\"abc\"");
        assert_eq!(t("'abc"), "'abc'");
    }

    fn c(w: &str) -> String {
        translate_word_checked(w)
    }

    #[test]
    fn checked_leaves_variable_free_words_alone() {
        for w in ["*.c", "a\\ b", "'$x'", "\"abc\"", "{a,b}c", "x{b}y", "~", "~/x", "=ls"] {
            assert_eq!(c(w), t(w), "{w}");
        }
    }

    #[test]
    fn checked_undefined_variable_guard() {
        // tcsh -f: `echo $x`, `echo ${x}`, `echo "$x"`, `echo $#x`,
        // `echo $%x`, `echo $x:h`, `echo $x[1]` all print
        // `x: Undefined variable.` and end the script with status 1.
        assert_eq!(c("\"$x\""), "\"${${x?Undefined variable.}:+}${x}\"");
        assert_eq!(c("$#x"), "${${x?Undefined variable.}:+}${#x}");
        for w in ["${x}", "$%x", "$x:h", "$x[1]", "\"a${x}b\""] {
            assert!(c(w).contains("${${x?Undefined variable.}:+}"), "{w}");
        }
        // tcsh: `$?x` is 0, `$1` with no argument is an empty word
        assert_eq!(c("$?x"), "${+x}");
        assert!(!c("$1").contains("Undefined"));
        assert!(!c("$argv").contains("Undefined"));
        assert!(!c("$status").contains("Undefined"));
    }

    #[test]
    fn checked_mapped_special_reports_the_csh_name() {
        // tcsh with HOME unset: `$home` is `home: Undefined variable.`
        assert!(c("$home").starts_with("${${HOME-${home?Undefined variable.}}:+}"));
        assert!(c("$term").starts_with("${${TERM-${term?Undefined variable.}}:+}"));
        // tcsh always defines these
        assert!(!c("$user").contains("Undefined"));
        assert!(!c("$cwd").contains("Undefined"));
    }

    #[test]
    fn checked_subscript_range_guard() {
        // tcsh, x=(a b c): `$x[4]`, `$x[3-4]`, `$x[0-2]`, `$x[-4]` and
        // `$x[4-4]` abort with `x: Subscript out of range.`
        let msg = "${__csh?x: Subscript out of range.}";
        assert!(c("$x[4]").contains(&format!("${{${{${{:-$((4 > ${{#x}}))}}#0}}:+{msg}}}")));
        assert!(c("$x[3-4]").contains("$((4 > ${#x}))"));
        assert!(c("$x[-4]").contains("$((4 > ${#x}))"));
        assert!(c("$x[0-2]").contains("$((2 != 0))"));
        // `$x[0]`, `$x[4-]`, `$x[3-1]`, `$x[-]`, `$x[*]`, `$x[2-3]` never
        // abort on a size check beyond the upper bound
        assert!(!c("$x[0]").contains("Subscript"));
        assert!(!c("$x[4-]").contains("Subscript"));
        assert!(!c("$x[-]").contains("Subscript"));
        assert!(!c("$x[*]").contains("Subscript"));
        // tcsh, x=(a b c): `$x[0-]` aborts, `$e[0-]` with e=() does not
        assert!(c("$x[0-]").contains("$((${#x} != 0))"));
    }

    #[test]
    fn checked_subscript_bounds_from_variables() {
        // tcsh, x=(a b c): i=4 aborts, i=0 gives an empty word, i unset is
        // `i: Undefined variable.`
        let out = c("$x[$i]");
        assert!(out.contains("${${i?Undefined variable.}:+}"));
        assert!(out.contains("$((${i} > ${#x}))"));
        let out = c("$x[$i-$j]");
        assert!(out.contains("${${j?Undefined variable.}:+}"));
        assert!(out.contains("$((${j} > ${#x} || (${i} == 0 && ${j} != 0)))"));
    }

    #[test]
    fn positional_names_take_no_subscript() {
        // tcsh, argv=(a b): `echo $1[2]` and `echo $*[3]` are `No match.`:
        // the value followed by the glob text `[2]`.
        assert_eq!(t("$1[2]"), "${=1}[2]");
        assert_eq!(t("$*[3]"), "${=argv}[3]");
        // `$argv[2]` is a real subscript
        assert_eq!(t("$argv[2]"), "${=argv[2]}");
    }

    #[test]
    fn count_of_environment_variable_is_its_value() {
        // tcsh: `setenv E 'a b'; echo $#E` prints `a b` (two words);
        // `set E = (a b c); echo $#E` prints 3.
        let out = t("$#E");
        assert!(out.contains("${${(M)${(t)E}:#array*}:+${#E}}"));
        assert!(out.contains("${${${(t)E}:#array*}:+${=E}}"));
        assert_eq!(t("\"$#E\""), "\"${${(M)${(t)E}:#array*}:+${#E}}${${${(t)E}:#array*}:+${E}}\"");
        // lower-case names are csh lists: plain element count
        assert_eq!(t("$#x"), "${#x}");
        // the mapped specials are one-word lists
        assert_eq!(t("$#home"), "1");
        assert_eq!(t("$#cwd"), "1");
    }

    #[test]
    fn checked_unknown_user() {
        // tcsh: `echo ~nouser/x` is `Unknown user: nouser.`
        let out = c("~nouser/x");
        assert!(out.contains("${__csh?Unknown user: nouser.}"));
        assert!(out.ends_with("\"$(print -r -- ~nouser)\"/x"));
        // the user probe runs before the directory is expanded
        assert!(out.starts_with("${${$({ : ~nouser; } 2>/dev/null && print known):-"));
        // bare `~`, `~/x` and a quoted `~nouser` are not lookups
        assert_eq!(c("~/x"), "~/x");
        assert_eq!(c("\"~nouser\""), "\"~nouser\"");
    }

    #[test]
    fn checked_brace_with_multi_word_variable() {
        // tcsh, v=(a b): `{$v,z}`, `x{$v}y` and `{$v:q,z}` are
        // `Missing '}'.`; `{"$v",z}` and `"{$v,z}"` are fine.
        for w in ["{$v,z}", "x{$v}y", "{$v:q,z}"] {
            assert!(c(w).contains("${__csh?Missing '}'.}"), "{w}");
        }
        assert!(!c("{\"$v\",z}").contains("Missing"));
        assert!(!c("\"{$v,z}\"").contains("Missing"));
    }

    #[test]
    fn checked_globs_unquoted_values_only() {
        // tcsh: `set x = '*.c'; echo $x` lists the files; `"$x"`, `$x:q`
        // and `$x:x` print `*.c`.
        let out = c("$x");
        assert!(out.contains("${~${(@)"));
        assert!(out.contains("(\\~*|*[*?\\[\\{]*)"));
        for w in ["\"$x\"", "$x:q", "$x:x"] {
            assert!(!c(w).contains("${~"), "{w}");
        }
        // the plain entry point never globs
        assert_eq!(t("$x"), "${=x}");
    }

    #[test]
    fn checked_whole_word_backquote_in_quotes() {
        // tcsh: `printf '<%s>' A "`printf ''`" B` is <A><B>; zsh keeps one
        // empty word for the quoted expansion, so it is left unquoted.
        assert_eq!(c("\"`ls`\""), "${(@)${(@f)\"$(ls)\"}:#}");
        // glued to other text it must stay quoted: tcsh `"x`printf ''`y"` is xy
        assert_eq!(c("\"x`ls`y\""), "\"x${(@)${(@f)\"$(ls)\"}:#}y\"");
        assert_eq!(t("\"`ls`\""), "\"${(@)${(@f)\"$(ls)\"}:#}\"");
    }
}
