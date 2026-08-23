//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! C resolves the quoting inside a `[...]` subscript by RE-LEXING the
//! subscript text: `getindex` (Src/params.c:2022) calls
//! `parse_subscript(s, scanflags & SCANPM_DQUOTED, ']')` at c:2029, which
//! untokenizes the text and pushes it back through the lexer
//! (`dquote_parse`, Src/lex.c:1751-1769). zshrs has no equivalent step on
//! that path — its `lex::parse_subscript` port throws away the tokenized
//! text C copies back at c:Src/lex.c:1772, and re-entering the real lexer
//! from inside paramsubst (and from the compiler, which resolves literal
//! assignment keys at compile time) would mean re-entrant lexer state on
//! the hottest expansion path.
//!
//! So the three C stages that decide what a backslash inside a subscript
//! MEANS — `dquote_parse`'s backslash arm, `getarg`'s marker disposition,
//! and the `remnulargs` / `parsestr` + `singsub` round that follows — are
//! expressed here as the one string transform they add up to. Each rule
//! cites the C line it comes from; see [`subscript_unescape`].
//!
//! Both callers are the "what key is this?" sites:
//!   * `ported::subst::paramsubst`  — `${A[\[k\]]}` (read)
//!   * `extensions::compile_zsh::compile_assign` — `A[\[k\]]=v` (store)

use crate::ported::zsh_h::{Bnull, Qstring, Qtick, Stringg, Tick};

/// Backslash disposition inside a `[...]` subscript — the net effect of
/// the re-lex C runs on a subscript's SOURCE text.
///
/// `getindex` NEVER reads the subscript the way the outer lexer left it.
/// It calls `parse_subscript(s, scanflags & SCANPM_DQUOTED, ']')`
/// (c:Src/params.c:2029), and `parse_subscript` untokenizes the text and
/// re-lexes it through `dquote_parse(']', sub)`
/// (c:Src/lex.c:1751-1769 — `untokenize(t = dupstring_wlen(s, l));
/// inpush(t, 0, NULL); … err = dquote_parse(endchar, sub);`). That
/// re-lex is where a backslash inside a subscript acquires its meaning:
///
/// ```text
/// c:Src/lex.c:1497-1512
///     if (c != '\n') {
///         if (c == '$' || c == '\\' || (c == '}' && !intick && bct) ||
///             c == endchar || c == '`' ||
///             (endchar == ']' && (c == '[' || c == ']' ||
///                                 c == '(' || c == ')' ||
///                                 c == '{' || c == '}' ||
///                                 (c == '"' && sub))))
///             add(Bnull);
///         else {
///             /* lexstop is implicitly handled here */
///             add('\\');
///             goto cont;
///         }
///     } else if (sub || unset(CSHJUNKIEQUOTES) || endchar != '"')
///         continue;
/// ```
///
/// With `endchar == ']'` a backslash before one of ``$ \ ` ] [ ( ) { }``
/// (plus `"` when the subscript is inside double quotes, `sub`) becomes
/// the `Bnull` marker + the literal char; a backslash before ANY other
/// char stays a literal backslash. That asymmetry is exactly why
/// `A[\[k\]]` keys on `[k]` while `A[a\ b]` / `A[a\*b]` keep theirs.
/// Backslash-newline is dropped outright (c:1513).
///
/// `getarg` then disposes of the markers (c:Src/params.c:1538-1551):
///
/// ```text
///     if (inull(c)) {
///         c = t[1];
///         if (c == '[' || c == ']' || c == '(' || c == ')' ||
///             c == '{' || c == '}') {
///             if (ishash && i) *t = ztokens[*t - Pound];
///             needtok = 1; ++t;
///         } else if (c != '"')
///             *t = ztokens[*t - Pound];
///         continue;
///     }
/// ```
///
/// — a marker before a bracket/paren/brace (or before `"`) is KEPT and
/// later DELETED by `remnulargs` (c:1583-1584, hash key path), so the
/// escaped bracket reaches the hash table bare. Every other marker is
/// untokenized back to a literal `\` (`ztokens[Bnull - Pound]` is `\`,
/// c:Src/lex.c:38), which the `parsestr` + `singsub` round at
/// c:1585-1593 re-marks and drops one stage later — so ``\$``, `\\` and
/// ``\` `` also lose their backslash, just further down the pipeline.
///
/// zshrs has no equivalent re-lex step on this path (its
/// `lex::parse_subscript` discards the tokenized text C copies back at
/// c:Src/lex.c:1772), so a source-literal backslash reached the assoc
/// key verbatim: `A[\[k\]]=v` stored the 5-char key `\[k\]` where zsh
/// stores `[k]`. This function is that missing step, expressed as the
/// composite string transform the three C stages add up to.
///
/// * `sub` — C's `SCANPM_DQUOTED`: the subscript sits inside `"…"`.
/// * `resolve_dollar` — the caller has NO `parsestr`/`singsub` round
///   after this call (compile-time literal key), so apply that stage's
///   share of the work here as well.
///
/// Returns the rewritten text and whether an UNESCAPED `$` / `` ` ``
/// (i.e. a live expansion, which C resolves in `singsub` at c:1592)
/// is still present.
pub fn subscript_unescape(s: &str, sub: bool, resolve_dollar: bool) -> (String, bool) {
    // Fast path: no backslash and no expansion char — C's re-lex is an
    // identity transform on such text.
    if !s.contains('\\')
        && !s.contains('$')
        && !s.contains('`')
        && !s.contains(Stringg)
        && !s.contains(Qstring)
        && !s.contains(Tick)
        && !s.contains(Qtick)
    {
        return (s.to_string(), false);
    }
    let mut out = String::with_capacity(s.len());
    let mut live = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.peek().copied() {
                // c:Src/lex.c:1497 — `if (c != '\n')`; the else arm at
                // c:1513 `continue`s for endchar != '"', dropping both.
                Some('\n') => {
                    it.next();
                }
                // c:Src/lex.c:1503-1506 (the `endchar == ']'` set) +
                // c:Src/params.c:1541-1548 (marker kept) +
                // c:Src/params.c:1584 remnulargs (marker deleted).
                Some(n)
                    if matches!(n, '[' | ']' | '(' | ')' | '{' | '}') || (sub && n == '"') =>
                {
                    out.push(n);
                    it.next();
                }
                // c:Src/lex.c:1501 (`$`, `\`, backtick) +
                // c:Src/params.c:1549-1550 (marker → literal `\`) +
                // c:Src/params.c:1585-1592 parsestr/singsub (re-marked,
                // then dropped by prefork's remnulargs at c:169).
                Some(n) if resolve_dollar && matches!(n, '$' | '\\' | '`') => {
                    out.push(n);
                    it.next();
                }
                // c:Src/lex.c:1508-1511 — `add('\\'); goto cont;`: the
                // backslash is ordinary text and the escaped char is
                // copied verbatim.
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                    it.next();
                }
                // Trailing lone backslash: C hits EOF inside
                // dquote_parse and errors out (c:1518 lexstop). Keep the
                // char so the caller's own error path decides.
                None => out.push('\\'),
            }
            continue;
        }
        // c:Src/params.c:1592 singsub — an UNESCAPED `$`/backtick (in
        // either ASCII or lexer-token spelling) is a live expansion.
        if c == '$' || c == '`' || c == Stringg || c == Qstring || c == Tick || c == Qtick {
            live = true;
        }
        out.push(c);
    }
    (out, live)
}

/// Same C stages as [`subscript_unescape`], stopped one step earlier and
/// re-encoded for a caller that still has to run C's `parsestr` + `singsub`
/// round (c:Src/params.c:1585-1592) through the word compiler.
///
/// [`subscript_unescape`] returns PLAIN text, which is right for a key the
/// caller stores verbatim but wrong for a key that still holds a live
/// expansion: its resolved `$` would be re-expanded by the word compiler and
/// its now-bare `[` would be read as a glob. C never has that problem because
/// its intermediate text is MARKED — `getarg` keeps the `Bnull` before a
/// bracket (c:1541-1548) and writes a literal `\` before the others
/// (c:1549-1550), and `parsestr` re-marks those (c:1588) before `singsub`
/// expands what is left. zshrs's word compiler consumes the same lexer
/// encoding, so emit the marker directly and let it do `singsub`'s job:
///
/// | source | C intermediate                        | emitted here |
/// |--------|---------------------------------------|--------------|
/// | `\[` `\]` `\(` `\)` `\{` `\}` (and `\"` when `sub`) | marker kept (c:1547), deleted by `remnulargs` (c:1583) | `Bnull` + char |
/// | `\$` `\\` `` \` ``                    | marker → `\` (c:1550), re-marked by `parsestr` (c:1588), dropped by `singsub`'s `prefork`/`remnulargs` (c:Src/subst.c:169) | `Bnull` + char |
/// | any other `\X`                        | ordinary text (c:Src/lex.c:1510 `add('\\')`) — survives BOTH re-lexes because the second one runs with `endchar == '\0'` and never marks `X` | `\` + char |
/// | `\` + newline                         | dropped (c:Src/lex.c:1513)            | — |
/// | everything else                       | untouched                             | verbatim |
///
/// An unescaped `$` / `` ` `` is deliberately left live — that is exactly the
/// work c:1592 `singsub` still has to do.
///
/// * `sub` — C's `SCANPM_DQUOTED`: the subscript sits inside `"…"`.
pub fn subscript_escape_markers(s: &str, sub: bool) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.peek().copied() {
                // c:Src/lex.c:1513
                Some('\n') => {
                    it.next();
                }
                // c:Src/lex.c:1501-1506 — the marked set for endchar == ']'.
                Some(n)
                    if matches!(n, '[' | ']' | '(' | ')' | '{' | '}' | '$' | '\\' | '`')
                        || (sub && n == '"') =>
                {
                    out.push(Bnull);
                    out.push(n);
                    it.next();
                }
                // c:Src/lex.c:1508-1511 — `add('\\'); goto cont;`.
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                    it.next();
                }
                None => out.push('\\'),
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// !!! WARNING: RUST-ONLY HELPER !!!
/// C has no separate function here: this is the scan loop that OPENS
/// `getarg` (c:Src/params.c:1533-1541), lifted out because the Rust
/// paramsubst expands the whole subscript in one place and then needs
/// to know where C would have cut it.
///
/// c:Src/params.c:1533-1541 —
///     for (t = s, i = 0;
///          (c = *t) &&
///              ((c != Outbrack && (ishash || c != ',')) || i || inpar);
///          t++) {
///         /* Untokenize inull() except before brackets and double-quotes */
///         if (inull(c)) { c = t[1]; if (c == '[' || … ) { … ++t; } … continue; }
///         if (c == '[' || c == Inbrack) i++;
///         else if (c == ']' || c == Outbrack) i--;
///         if (c == '(' || c == Inpar) inpar++;
///         else if (c == ')' || c == Outpar) inpar--;
///         …
///     }
///
/// Returns the two RAW (still unexpanded) argument texts when the scan
/// stopped on a top-level range comma, else None (one argument only).
/// `ishash` mirrors C's `ishash` gate: for a hash, `,` is an ordinary
/// key byte and never terminates the argument.
pub fn subscript_arg_split(s: &str, ishash: bool) -> Option<(String, String)> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0_i32; // c:1512 — bracket nesting
    let mut inpar = 0_i32; // c:1512
    let mut k = 0_usize;
    while k < chars.len() {
        let c = chars[k];
        // c:1543-1552 — `if (inull(c))`: the marker's NEXT char decides.
        // Before a bracket/brace/paren the pair is skipped wholesale (c:1549
        // `++t`), so an escaped bracket never moves the nesting counters.
        // zshrs also sees the SOURCE-literal spelling of the escape (`\`)
        // because it has no `parse_subscript` re-lex; treat both alike.
        if c == crate::ported::zsh_h::Bnull
            || c == crate::ported::zsh_h::Bnullkeep
            || c == '\\'
        {
            if let Some(&n) = chars.get(k + 1) {
                if matches!(n, '[' | ']' | '(' | ')' | '{' | '}')
                    || n == crate::ported::zsh_h::Inbrack
                    || n == crate::ported::zsh_h::Outbrack
                    || n == crate::ported::zsh_h::Inpar
                    || n == crate::ported::zsh_h::Outpar
                    || n == crate::ported::zsh_h::Inbrace
                    || n == crate::ported::zsh_h::Outbrace
                {
                    k += 2; // c:1549 `++t` plus the loop's own `t++`
                    continue;
                }
            }
            k += 1; // c:1551 `continue` — the escaped char is examined next
            continue;
        }
        // c:1534 — `(c != Outbrack && (ishash || c != ','))`: a top-level
        // comma ends the argument unless the target is a hash.
        // `Comma` (c:Src/zsh.h) is the lexer TOKEN spelling of the same byte —
        // a nested `${…}` body reaches paramsubst tokenized, so testing only
        // the ASCII form reported "one argument" for `${(A@)a[1,2]}` and every
        // downstream site then treated the slice as a single element.
        if (c == ',' || c == crate::ported::zsh_h::Comma) && !ishash && i == 0 && inpar == 0 {
            return Some((
                chars[..k].iter().collect(),
                chars[k + 1..].iter().collect(),
            ));
        }
        match c {
            '[' | crate::ported::zsh_h::Inbrack => i += 1,      // c:1553
            ']' | crate::ported::zsh_h::Outbrack => i -= 1,     // c:1555
            '(' | crate::ported::zsh_h::Inpar => inpar += 1,    // c:1557
            ')' | crate::ported::zsh_h::Outpar => inpar -= 1,   // c:1559
            _ => {}
        }
        k += 1;
    }
    None
}

/// !!! WARNING: RUST-ONLY HELPER !!!
/// Resolve "is this subscript a range, and what are its bounds?" using
/// the parse-time decision recorded by `subscript_arg_split` when one is
/// available (c:Src/params.c:1533-1536 — C splits BEFORE expanding), and
/// falling back to a depth-0 comma scan of the already-expanded text for
/// the reference paths that do not record one.
pub fn subscript_range_bounds(
    sub: &str,
    known: &Option<(String, Option<(String, String)>)>,
) -> Option<(String, String)> {
    // The record is only authoritative for the exact subscript text it was
    // taken from: paramsubst reassigns `subscript` on several arms (the
    // `${!name}` prefix form, the magic-assoc key rebuild), and a stale
    // record must not answer for a different string.
    if let Some((recorded, split)) = known {
        if recorded == sub {
            return split.clone();
        }
    }
    let bs: Vec<char> = sub.chars().collect();
    let mut depth = 0_i32;
    for (k, &c) in bs.iter().enumerate() {
        match c {
            '(' | crate::ported::zsh_h::Inpar | '[' | crate::ported::zsh_h::Inbrack => depth += 1,
            ')' | crate::ported::zsh_h::Outpar | ']' | crate::ported::zsh_h::Outbrack => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            ',' if depth == 0 => {
                return Some((
                    bs[..k].iter().collect(),
                    bs[k + 1..].iter().collect(),
                ));
            }
            c if c == crate::ported::zsh_h::Comma && depth == 0 => {
                return Some((
                    bs[..k].iter().collect(),
                    bs[k + 1..].iter().collect(),
                ));
            }
            _ => {}
        }
    }
    None
}
