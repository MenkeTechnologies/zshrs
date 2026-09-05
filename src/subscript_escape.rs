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
                Some(n) if matches!(n, '[' | ']' | '(' | ')' | '{' | '}') || (sub && n == '"') => {
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
        if c == crate::ported::zsh_h::Bnull || c == crate::ported::zsh_h::Bnullkeep || c == '\\' {
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
            return Some((chars[..k].iter().collect(), chars[k + 1..].iter().collect()));
        }
        match c {
            '[' | crate::ported::zsh_h::Inbrack => i += 1, // c:1553
            ']' | crate::ported::zsh_h::Outbrack => i -= 1, // c:1555
            '(' | crate::ported::zsh_h::Inpar => inpar += 1, // c:1557
            ')' | crate::ported::zsh_h::Outpar => inpar -= 1, // c:1559
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
                return Some((bs[..k].iter().collect(), bs[k + 1..].iter().collect()));
            }
            c if c == crate::ported::zsh_h::Comma && depth == 0 => {
                return Some((bs[..k].iter().collect(), bs[k + 1..].iter().collect()));
            }
            _ => {}
        }
    }
    None
}

/// !!! WARNING: RUST-ONLY HELPER !!!
/// Inverse of [`subscript_unescape`]'s marked set, for the one place the port
/// has to hand an ALREADY-EXPANDED key back through a text subscript.
///
/// c:Src/subst.c:3312-3316 — the `${name[key]=value}` family assigns with
///     *idend = '\0';
///     Param pm = setsparam(idbeg, ztrdup(val));
/// i.e. C re-parses the flat `name[key]` text too. That is sound in C because
/// `idbeg` still holds the LEXER's spelling, where a `]` inside the key is a
/// `Bnull`-marked byte and cannot close the subscript. zshrs's paramsubst has
/// already resolved the subscript to plain text by then (`expand_sub_arg`), so
/// the rebuilt string `B[\\]]` re-parsed as key `\` — the assignment landed on
/// the wrong key and the read-back came up empty (D06subscript.ztst
/// "Associative array substitution-assignment with reverse pattern subscript
/// key"). Re-apply the escaping the re-parse will strip, exactly over the set
/// c:Src/lex.c:1501-1506 marks for `endchar == ']'`, so the round trip is the
/// identity.
///
/// Returns the input untouched for a FLAG-GROUP subscript (`(r)pat`), whose
/// parentheses are structure rather than data.
pub fn subscript_requote_for_assign(k: &str) -> std::borrow::Cow<'_, str> {
    let trimmed = k.trim_start();
    if trimmed.starts_with('(') || trimmed.starts_with(crate::ported::zsh_h::Inpar) {
        return std::borrow::Cow::Borrowed(k); // flag group: structural
    }
    // c:Src/lex.c:1501-1506 — the set that `dquote_parse(']')` marks.
    if !k.contains(|c| {
        matches!(
            c,
            '$' | '\\' | '`' | '[' | ']' | '(' | ')' | '{' | '}' | '"'
        )
    }) {
        return std::borrow::Cow::Borrowed(k);
    }
    let mut out = String::with_capacity(k.len() * 2);
    for c in k.chars() {
        if matches!(
            c,
            '$' | '\\' | '`' | '[' | ']' | '(' | ')' | '{' | '}' | '"'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    std::borrow::Cow::Owned(out)
}

/// !!! WARNING: RUST-ONLY HELPER !!!
/// Classification of ONE subscript operand — a range bound (`${a[lo,hi]}`)
/// or a chained subscript (`${a[lo,hi][SUB]}`) — whose text may open with a
/// `(...)` flag group.
///
/// C has no such function: `getarg` (c:Src/params.c:1367) parses the flags,
/// runs the search and returns the index all in one pass, writing its
/// side-effects back through `Value *v` / `int *inv` out-parameters. zshrs's
/// `ported::params::getarg` returns the matched ELEMENT for `r`/`R` and the
/// INDEX for `i`/`I` (see `getarg_out`), and it has no `Value` to record
/// `v->isarr |= SCANPM_WANTVALS` in — so the two facts every bound consumer
/// needs (the match POSITION and whether WANTVALS was raised) are recovered
/// here in one place instead of being re-derived at each call site.
pub enum SubscriptBound {
    /// c:Src/params.c:1729-1760 — the flag group ran a pattern SEARCH.
    /// `.0` is getarg's 1-based match index `r` (c:1758 returns 0 for a
    /// REVERSE miss, c:1751 `len + 1` for a FORWARD miss); `.1` records
    /// c:1523 `v->isarr |= SCANPM_WANTVALS`, raised by `r`/`R`/`k`/`K`
    /// when the `i`/`I` index flag (`ind`) is off.
    Search(i64, bool),
    /// c:Src/params.c:1597 `r = mathevalarg(s, &s)` — no search ran; the
    /// payload is the text to evaluate as arithmetic. A recognised but
    /// non-search flag group (`(s.X.)`, `(w)`, …) has been stripped; an
    /// UNKNOWN group is left in place because c:1477-1483's `flagerr` arm
    /// rewinds to before the `(` and re-reads the whole group as math.
    Math(String),
}

/// !!! WARNING: RUST-ONLY HELPER !!!
/// Classify one subscript operand against `arr` — see [`SubscriptBound`].
///
/// c:Src/params.c getindex — a bound with a search-flag subscript
/// (`(r)pat`/`(i)pat`) yields the INDEX of the match (the `*inv`/`*w` path),
/// not the value: `${a[(r)3,(r)5]}` slices between the matched positions.
/// `getarg` returns the value for `r`/`R` but the index for `i`/`I`. `r` is a
/// FORWARD first-match (c:1411 `down = 0`), `R` a REVERSE last-match (c:1416
/// `down = 1`), so map `r`→`i` / `R`→`I` to get the matching index in the SAME
/// direction — preserving forward/reverse for duplicate matches and the
/// no-match returns (forward no-match → len+1, reverse → 0).
pub fn subscript_bound_classify(t: &str, arr: &[String]) -> SubscriptBound {
    let t = t.trim();
    let close = match t.find(')') {
        Some(c) if t.starts_with('(') => c,
        _ => return SubscriptBound::Math(t.to_string()),
    };
    let flags = &t[1..close];
    if flags
        .chars()
        .any(|c| matches!(c, 'r' | 'R' | 'i' | 'I' | 'k' | 'K'))
    {
        // Search flag → matched INDEX via getarg (r/R are value-returning →
        // map to the i/I index form in the same direction).
        let mapped: String = flags
            .chars()
            .map(|c| match c {
                'r' => 'i',
                'R' => 'I',
                // On a non-hash, k/K are r/R (c:1400/1405 gate only
                // `keymatch` on ishash), so they need the same value→index
                // remap to serve as a range BOUND. Bug #1050.
                'k' => 'i',
                'K' => 'I',
                o => o,
            })
            .collect();
        // c:Src/params.c:1516-1531 — the `*inv` decision. `ind` is set only
        // by `i`/`I` (c:1420/1424); `rev` by `r`/`R`/`k`/`K`. With `ind` off
        // and `rev` on, c:1523 raises `v->isarr |= SCANPM_WANTVALS`, and that
        // bit rides on the Value into any CHAINED subscript (c:Src/subst.c:2896
        // `v->isarr = isarr`, where `isarr` is the whole scanflags mask), where
        // c:1515 `else if (v->isarr & SCANPM_WANTVALS) *inv = 0;` makes a
        // later `(i)`/`(I)` return the ELEMENT instead of the index.
        let ind = flags.contains('i') || flags.contains('I');
        let wantvals = !ind;
        let new_sub = format!("({}){}", mapped, &t[close + 1..]);
        if let Some(crate::ported::params::getarg_out::Value(v)) =
            crate::ported::params::getarg(&new_sub, Some(arr), None, None)
        {
            if let Ok(n) = v.to_str().trim().parse::<i64>() {
                return SubscriptBound::Search(n, wantvals);
            }
        }
        return SubscriptBound::Math(t.to_string());
    }
    // c:Src/params.c:1412 — `for (s++; *s != ')' && *s != Outpar && s != *str;
    // s++)`. An EMPTY group never enters the loop body, so it never reaches
    // `flagerr`; c:1506-1507 `if (s != *str) s++;` then steps past the `)` and
    // c:1618 `mathevalarg(s, &s)` reads the text AFTER it. So `()` parses and
    // is stripped exactly like `(e)` or `(s.X.)` — `a=(x y z); ${a[1,()2]}` is
    // `x y` in zsh, not a math error over `()2`. The scalar char-slice arm
    // (subst.rs `bound_idx`) already gates on a group that PARSED and so
    // already accepted the empty one; this copy tested only the FIRST char and
    // `None` failed that test, which was invisible while a failed
    // `mathevali` fell back to a default bound and merely widened the slice.
    if flags.chars().next().is_none_or(|c| {
        // c:Src/params.c:1392-1476 — the flag switch's cases, verbatim.
        matches!(
            c,
            'r' | 'R' | 'k' | 'K' | 'i' | 'I' | 'w' | 'f' | 'e' | 'n' | 'b' | 'p' | 's'
        )
    }) {
        // Separator/word flag (`(s.X.)` etc.) is a no-op for an integer
        // slice bound (c:#83); strip it and parse the remainder.
        return SubscriptBound::Math(t[close + 1..].to_string());
    }
    // c:Src/params.c:1477-1482 — anything else is NOT a flag group. C's flag
    // switch falls to
    //     default:
    //       flagerr:
    //         num = 1; word = rev = ind = down = keymatch = 0; sep = NULL;
    //         s = *str - 1;      /* rewind */
    // so an unknown flag char REWINDS to before the `(` and the group is
    // re-read as MATH. That is why `${arr[(zz)1]}` reports `bad math
    // expression` rather than a flag error.
    //
    // Stripping unconditionally deleted a PARENTHESISED range bound:
    // `${arr[(x), 4]}` left the text empty, so the bound fell back to the
    // default 1 and the slice became 1..4 (`a b c d`) where zsh gives
    // `b c d`. Handing `(x)` to mathevali below is C's behaviour. Only the
    // RANGE form was affected: a single `${arr[(x)]}` takes a different arm
    // and already evaluated as math.
    SubscriptBound::Math(t.to_string())
}
