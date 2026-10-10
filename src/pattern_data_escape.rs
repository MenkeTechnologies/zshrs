//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! The DATA half of docs/BUGS.md #1090: how a backslash that is a
//! CHARACTER OF A VALUE has to be spelled before it reaches
//! `ported::pattern::patcompile`.
//!
//! C never needs this transform. Its pattern compiler consumes the
//! LEXER's encoding, where a source-level quote already arrived as
//! `Bnull`/`Bnullkeep` + payload (c:Src/zsh.h:195-200), so a RAW
//! backslash in `patcompile`'s input can only be data. A substituted
//! value acquires its pattern meaning in `zshtokenize`
//! (c:Src/glob.c:3585-3653), reached from `strcatsub`'s
//! `if (glbsub) shtokenize(dest)` (c:Src/subst.c:822/830) for
//! `${~spec}` / `GLOB_SUBST`, and that function rewrites a backslash
//! into a quote marker ONLY when the next character reaches its
//! `ztokens` scan:
//!
//! ```text
//! c:Src/glob.c:3597-3605   case Bnull: case Bnullkeep: case '\\':
//!                              if (bslash) { s[-1] = … Bnullkeep/Bnull; break; }
//!                              bslash = 1; continue;
//! c:Src/glob.c:3640-3648   for (t = ztokens; *t; t++)
//!                              if (*t == *s) {
//!                                  if (bslash) s[-1] = … Bnullkeep/Bnull;
//!                                  else *s = (t - ztokens) + Pound;
//!                                  break;
//!                              }
//! c:Src/glob.c:3651        bslash = 0;
//! ```
//!
//! Before anything else — a space, a `$`, a `{` — no `switch` arm fires,
//! c:3651 just clears `bslash`, and BOTH bytes survive in the string as
//! ordinary literal data. That is why real zsh answers
//!
//! ```text
//! p='a\ b'; [[ 'a b'  == ${~p} ]]   # no match — the pattern holds a backslash
//! p='a\ b'; [[ 'a\ b' == ${~p} ]]   # match
//! ```
//!
//! `ported::pattern`'s input normalizer (src/ported/pattern.rs, the `\\`
//! arm) reads a lone raw `\X` as a QUOTE of X — the spelling every
//! SOURCE-level pattern path in zshrs hands it (the cond/case pattern
//! builder in `extensions::compile_zsh`, `${v//\%/%%}`'s builder in
//! `ported::subst`) — and spells a literal backslash as the pair `\\`.
//! So doubling exactly the backslashes `zshtokenize` declines to consume
//! is what carries C's `Bnull`-vs-raw split into the Rust encoding.
//! Backslashes the tokenizer WOULD consume are left in place so the
//! downstream tokenizer/normalizer still folds them into a quote at
//! their original position.
//!
//! Callers are the "this pattern text came out of a VALUE" sites:
//!   * `ported::subst::paramsubst` — the search-subscript patterns
//!     (`${a[(I)…]}` / `(i)` / `(r)` / `(R)` / `(K)`), which reach
//!     `patcompile` through `tokenize` alone (c:Src/params.c:1727).
//!   * `fusevm_bridge`'s `BUILTIN_GLOB_SUBST_GUARD` /
//!     `BUILTIN_PAT_DATA_BACKSLASH` — the `${~spec}` and `setopt
//!     globsubst` legs of a `[[ … == pat ]]` RHS and a `case` arm, the
//!     `strcatsub` `shtokenize` C runs at c:Src/subst.c:822/830.

/// The `switch` labels of `zshtokenize` that can consume a preceding
/// backslash — c:Src/glob.c:3599 (`\\`), c:3606 (`<`), c:3623-3625
/// (`(`/`|`/`)`) and c:3629-3639 (`>`/`^`/`#`/`~`/`[`/`]`/`*`/`?`/`=`/
/// `-`/`!`).
///
/// A character that is in the `ztokens` TABLE (c:Src/lex.c:38) but has
/// NO `switch` label — `$`, `{`, `}`, `` ` ``, `,`, `'`, `"` — never
/// reaches the c:3640 scan, so its backslash stays data. zsh answers 2,
/// not 1, for
/// ```text
/// a=('a$b' 'a\$b'); q='a\$b'; print ${a[(I)$q]}
/// ```
fn quotes_a_metachar(c: char) -> bool {
    matches!(
        c,
        '<' | '('
            | '|'
            | ')'
            | '>'
            | '^'
            | '#'
            | '~'
            | '['
            | ']'
            | '*'
            | '?'
            | '='
            | '-'
            | '!'
            | '\\'
    )
}

/// Rewrite a pattern string that came out of a VALUE so
/// `ported::pattern`'s normalizer reads its backslashes the way
/// `zshtokenize` does — see the module docs.
///
/// Every backslash `zshtokenize` would NOT consume (c:Src/glob.c:3651
/// `bslash = 0` with nothing rewritten) is doubled, which is the
/// normalizer's literal-backslash form. A trailing lone backslash is
/// data too (c:3590 `for (; *s; s++)` ends before any arm can fire) and
/// is doubled as well.
pub fn escape_data_backslashes(v: &str) -> String {
    if !v.contains('\\') {
        return v.to_string();
    }
    let cs: Vec<char> = v.chars().collect();
    let mut out = String::with_capacity(v.len() + 4);
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c != '\\' {
            out.push(c);
            i += 1;
            continue;
        }
        if cs.get(i + 1).copied().is_some_and(quotes_a_metachar) {
            // c:3600-3602 / c:3642-3643 — the escape is honored; leave the
            // pair for the tokenizer to fold into `Bnull`/`Bnullkeep`.
            out.push(c);
            out.push(cs[i + 1]);
            i += 2;
        } else {
            // c:3651 `bslash = 0` with nothing rewritten — a data backslash.
            out.push('\\');
            out.push('\\');
            i += 1;
        }
    }
    out
}

/// The same DATA-vs-QUOTE split as [`escape_data_backslashes`], applied one
/// step LATER: to a string that has ALREADY been through `tokenize()` +
/// `remnulargs()` and is about to be handed straight to `patcompile`.
///
/// `zshtokenize` (c:Src/glob.c:3585, flags 0) folds every backslash it
/// honors into a `Bnull` written at the BACKSLASH's own position, leaving
/// the escaped character raw (c:3600-3602 `s[-1] = … Bnull` and c:3642-3643
/// for the `ztokens` scan). `remnulargs` (c:Src/glob.c:3658) then deletes
/// those `Bnull`s. So in the string C finally hands to `patcompile` every
/// surviving raw `\` is a literal backslash CHARACTER: `patcomppiece` has
/// no `case '\\'` at all (c:Src/pattern.c:1579-1601 — only `case Bnullkeep`
/// at c:1589 is special), so that byte compiles as ordinary text. This is
/// why real zsh answers
///
/// ```text
/// # the pattern is `\` + `(`, so it needs a literal backslash in the word
/// compset -P '\\\('     # PREFIX='('   -> no match
/// compset -P '\('       # PREFIX='('   -> match
/// ```
///
/// `ported::pattern`'s normalizer instead reads a raw `\X` as a QUOTE of X
/// (src/ported/pattern.rs, the `\\` arm) and spells a literal backslash as
/// the pair `\\`. Doubling every remaining backslash is therefore the exact
/// transposition of C's post-`remnulargs` encoding into the Rust one.
///
/// Distinct from [`escape_data_backslashes`], which runs BEFORE `tokenize`
/// and must leave a `\<metachar>` pair alone so the tokenizer can still fold
/// it into a quote. Here the tokenizer has already run, so no pair is
/// pending and every backslash left is data.
pub fn escape_tokenized_data_backslashes(v: &str) -> String {
    if !v.contains('\\') {
        return v.to_string();
    }
    v.replace('\\', "\\\\")
}

/// The SH_GLOB half of the same `strcatsub` step: `shtokenize` builds its
/// flags from the option (c:Src/glob.c:3575-3580)
///
/// ```text
/// int flags = ZSHTOK_SUBST;
/// if (isset(SHGLOB))
///     flags |= ZSHTOK_SHGLOB;
/// ```
///
/// and `zshtokenize` then DECLINES to tokenize `(`, `|` and `)`
/// (c:Src/glob.c:3617-3620)
///
/// ```text
/// case '(':
/// case '|':
/// case ')':
///     if (flags & ZSHTOK_SHGLOB)
///         break;
/// ```
///
/// so under SH_GLOB those three characters stay ordinary data. zsh applies
/// that unconditionally — even a KSH_GLOB group loses its meaning:
/// `zsh -fc 'setopt shglob kshglob; v="  x  "; print "[${v##+([[:space:]])}]"'`
/// prints `[  x  ]`, unchanged.
///
/// zshrs cannot express the suppression by skipping its own tokenize pass,
/// because the consumers tokenize the ASSEMBLED word once, after the value
/// has been concatenated with any source-level pattern text around it;
/// skipping there would de-meta the source half too. Spelling the three
/// characters in the normalizer's literal form (`\X`) instead carries the
/// suppression on exactly the bytes it belongs to.
///
/// `keep_ksh_groups` is the bash/ksh DROP-IN exemption — see
/// [`dropin_keeps_ksh_groups`]. With it set, a `(` that opens a ksh-style
/// extended group (`@(`, `*(`, `+(`, `?(`, `!(`) keeps its meaning, and so
/// do that group's `|` separators and its closing `)`; every other paren
/// and every `|` outside such a group is still literal, which is precisely
/// how bash and ksh read them. Groups nest, so the decision is stacked.
///
/// Characters inside a `[…]` class are left alone: they are class members
/// under every one of these rules, and `zshtokenize` — a flat scan — would
/// not have touched them either.
///
/// A backslash pair is stepped over whole, as in [`escape_data_backslashes`],
/// so an escape that is already present is never re-escaped into a literal
/// backslash plus a live metacharacter.
pub fn escape_shglob_parens(v: &str, keep_ksh_groups: bool) -> String {
    if !v.contains(['(', '|', ')']) {
        return v.to_string();
    }
    let cs: Vec<char> = v.chars().collect();
    let mut out = String::with_capacity(v.len() + 4);
    // One entry per open `(`: true when that group survives as a real
    // ksh-glob group, false when its parens were made literal.
    let mut groups: Vec<bool> = Vec::new();
    let mut in_class = false;
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' && i + 1 < cs.len() {
            out.push(c);
            out.push(cs[i + 1]);
            i += 2;
            continue;
        }
        if in_class {
            if c == ']' {
                in_class = false;
            }
            out.push(c);
            i += 1;
            continue;
        }
        match c {
            '[' => {
                in_class = true;
                out.push(c);
            }
            '(' => {
                let ksh = keep_ksh_groups
                    && i > 0
                    && matches!(cs[i - 1], '@' | '*' | '+' | '?' | '!')
                    && !(i >= 2 && cs[i - 2] == '\\');
                groups.push(ksh);
                if !ksh {
                    out.push('\\');
                }
                out.push(c);
            }
            ')' => {
                // An unmatched `)` closes nothing, so it is ordinary text.
                if !groups.pop().unwrap_or(false) {
                    out.push('\\');
                }
                out.push(c);
            }
            '|' => {
                if !groups.last().copied().unwrap_or(false) {
                    out.push('\\');
                }
                out.push(c);
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// Whether SH_GLOB's `(` / `|` / `)` suppression applies at all right now.
///
/// The zsh rule is the option, so this is simply `isset(SHGLOB)` — EXCEPT in
/// a bare Korn drop-in.
///
/// !!! DROP-IN GATE — no zsh C counterpart !!!
/// `zshrs --ksh` reaches EMULATE_KSH, which raises SH_GLOB, but its
/// reference is ksh, and ksh reads a BARE `(` as a grouping character in its
/// own right — `ksh -c 'v="-a"; print -r -- "[${v##-(a|b*)}]"'` prints `[]`,
/// i.e. the group matched and the whole value was stripped, where zsh under
/// SH_GLOB leaves `-(a|b*)` as six literal characters. So the Korn drop-in
/// suppresses nothing.
///
/// bash is the middle case and is handled by [`dropin_keeps_ksh_groups`]:
/// `(` is special there ONLY after `@ * + ? !`.
///
/// False for none of `--zsh`, native zshrs, or a zsh user's own
/// `emulate sh` / `emulate ksh` beyond what the option itself says — those
/// keep zsh's answer.
pub fn shglob_hides_parens() -> bool {
    crate::ported::zsh_h::isset(crate::ported::zsh_h::SHGLOB)
}

/// Whether a bash DROP-IN is running with extended patterns on.
///
/// !!! DROP-IN GATE — no zsh C counterpart !!!
/// `zshrs --bash` reaches EMULATE_SH, which raises SH_GLOB, but its
/// reference is bash. bash gives `@(…)` / `+(…)` their extended meaning in
/// exactly the positions zsh suppresses
/// (`bash -c 'shopt -s extglob; v="  x  "; echo "[${v##+([[:space:]])}]"'`
/// prints `[x  ]`, where zsh prints `[  x  ]`), while still reading a BARE
/// `(` as ordinary text — bash makes `(` special only after `@ * + ? !`.
/// So the bash drop-in keeps the ksh groups and literalizes everything else.
///
/// False in `--zsh`, in native zshrs, and under a zsh user's own
/// `emulate sh`, where zsh's answer is the correct one. bash's `extglob`
/// shopt is zshrs's `kshglob` (src/extensions/dash_mode.rs SHOPT table), so
/// the option carries the enable.
pub fn dropin_keeps_ksh_groups() -> bool {
    (crate::dash_mode::bash_mode() || crate::dash_mode::korn_mode())
        && crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHGLOB)
}

/// Whether a SOURCE-level `[[ … ]]` / `case` pattern must follow the
/// EMULATED shell's paren rules instead of zsh's.
///
/// !!! DROP-IN GATE — no zsh C counterpart !!!
/// zsh's answer for such a pattern comes from WHEN it was tokenized: the
/// parser turned `(` into a grouping token before SH_GLOB was set, so the
/// option later strands the `)` and the pattern is bad (c:Src/pattern.c:
/// 500-510 + :913-917). That history is a zsh artifact. A bare POSIX-family
/// drop-in has no such history — bash, ksh, dash and sh decide what `(`
/// means when they match, so a pattern zsh rejects is simply ordinary text
/// there:
/// ```text
/// bash -c "[[ '-a' = -(a|b*) ]] && echo M || echo N"   # N
/// ksh  -c "[[ '-a' = -(a|b*) ]] && echo M || echo N"   # N
/// zsh  -fc "setopt shglob; [[ '-a' = -(a|b*) ]]"       # bad pattern
/// ```
/// [`dropin_keeps_ksh_groups`] then decides whether `@(…)` / `+(…)` survive
/// inside that text.
///
/// `posix_faithful` is what separates the bare drop-in from the zsh-STYLE
/// leg (`--sh --zsh`, or a zsh user typing `emulate sh`), which must keep
/// zsh's answer.
pub fn dropin_source_pattern_parens_literal() -> bool {
    crate::dash_mode::posix_faithful() && crate::ported::zsh_h::isset(crate::ported::zsh_h::SHGLOB)
}

/// !!! WARNING: RUST-ONLY HELPER — NO C COUNTERPART !!!
///
/// Fills `ported::subst::DEFAULT_WORD_GLOB_BARS` for the default / alternate
/// word of `${x:-word}` / `${x-word}` / `${x:+word}` / `${x+word}`. In C the
/// lexer makes a `|` inside `${…}` the `Bar` token (c:Src/lex.c:1000-1008),
/// multsub keeps the token in the substituted value (c:Src/subst.c:3207-3228),
/// and globlist then reads a top-level alternation in the assembled word:
/// `${x:-b|a}` matches the file `b`, `X${x:-b|a}` is the pattern `Xb` | `a`.
/// zshrs untokenizes the word before BUILTIN_DEFAULT_WORD_GLOB, which puts
/// the recorded `Bar`s back.
///
/// `word` is the SOURCE text of the default / alternate word as paramsubst
/// holds it: an unquoted `|` is a raw `|`, while a quoted (`"b|a"`,
/// `'b|a'`) or escaped (`b\|a`) one arrives as `Bnull |` and stays literal.
/// A raw `|` inside `(…)` or `[…]` is left to the glob engine's own
/// grouping. `value` is what the word substituted to. Only a word with no
/// substitution syntax is recorded: its source text IS the value, so the
/// `|` positions are known without running multsub again (a `$(…)` in the
/// word must not run twice).
///
/// Returns whether the word holds a top-level source `|`, so the caller can
/// arm the default-word glob for it.
pub fn note_default_word_bars(word: &str, value: &str) -> bool {
    use crate::ported::zsh_h::{
        Bar, Bnull, Bnullkeep, Dnull, Inbrack, Inpar, Nularg, Outbrack, Outpar, Qstring, Qtick,
        Snull, Tick,
    };
    let cs: Vec<char> = word.chars().collect();
    let mut barred = String::with_capacity(word.len());
    let mut paren = 0i32;
    let mut bracket = false;
    let mut found = false;
    let mut substitutes = false;
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if (c == Bnull || c == Bnullkeep) && i + 1 < cs.len() {
            barred.push(cs[i + 1]); // quoted / escaped: literal
            i += 2;
            continue;
        }
        if c == Snull || c == Dnull || c == Nularg {
            i += 1;
            continue;
        }
        if matches!(c, '$' | '`' | '\u{e185}') || c == Qstring || c == Tick || c == Qtick {
            substitutes = true;
        }
        match c {
            '(' | Inpar => paren += 1,
            ')' | Outpar => paren = (paren - 1).max(0),
            '[' | Inbrack => bracket = true,
            ']' | Outbrack => bracket = false,
            _ => {}
        }
        if (c == '|' || c == Bar) && paren == 0 && !bracket {
            found = true;
            barred.push(Bar);
            i += 1;
            continue;
        }
        if ('\u{e184}'..='\u{e19c}').contains(&c) {
            barred.push_str(&crate::ported::lex::untokenize(&c.to_string()));
        } else {
            barred.push(c);
        }
        i += 1;
    }
    if found && !substitutes && crate::ported::lex::untokenize(&barred) == value {
        crate::ported::subst::DEFAULT_WORD_GLOB_BARS
            .with(|b| b.borrow_mut().push((value.to_string(), barred)));
    }
    found
}

/// Engine selected by the options of a ksh93 `~(options)` / `~(options:pat)`
/// pattern prefix.
#[derive(Clone, Copy, PartialEq)]
enum Ksh93PatEngine {
    /// ksh glob (`K`, the default; `X`/`N` read the same here).
    Glob,
    /// `F`: fixed string, found anywhere in the subject.
    Fixed,
    /// `E` / `P`: extended regular expression, searched anywhere in the subject.
    Ere,
    /// `G`: basic regular expression, searched anywhere in the subject.
    Bre,
}

/// Options parsed from `~(…)`: `(case-insensitive, engine)`, or `None` for a
/// letter this port does not model. `-` turns the letters after it off, `+`
/// on again.
fn ksh93_pat_options(flags: &[char]) -> Option<(bool, Ksh93PatEngine)> {
    let (mut icase, mut engine, mut off) = (false, Ksh93PatEngine::Glob, false);
    for &c in flags {
        match c {
            '-' => off = true,
            '+' => off = false,
            'i' => icase = !off,
            'F' if !off => engine = Ksh93PatEngine::Fixed,
            'E' | 'P' if !off => engine = Ksh93PatEngine::Ere,
            'G' if !off => engine = Ksh93PatEngine::Bre,
            'K' | 'X' | 'N' | 'F' | 'E' | 'P' | 'G' => {}
            _ => return None,
        }
    }
    Some((icase, engine))
}

/// Index just past the `)` closing the group whose `(` is at `open`.
fn ksh93_pat_group_end(cs: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = open;
    while i < cs.len() {
        match cs[i] {
            '\\' => i += 1,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// `ab`-style swap: the other-case twin of an ASCII letter.
fn ksh93_pat_swapcase(c: char) -> char {
    if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }
}

/// Rewrite the glob `cs` (normalized) / `orig` (as given) so that its letters
/// match either case: `a` -> `[aA]`, `[a-c]` -> `[a-cA-C]`.
fn ksh93_pat_fold_case(orig: &[char], cs: &[char], out: &mut String) {
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '\\' => {
                out.push(orig[i]);
                if let Some(&n) = orig.get(i + 1) {
                    out.push(n);
                }
                i += 2;
            }
            '[' => {
                // A class runs to the first `]` after at least one member.
                let start = i + 1 + usize::from(matches!(cs.get(i + 1), Some('!' | '^')));
                match cs[(start + 1).min(cs.len())..].iter().position(|&c| c == ']') {
                    Some(rel) => {
                        let close = start + 1 + rel;
                        out.extend(&orig[i..close]);
                        out.extend(cs[start..close].iter().filter(|c| c.is_ascii_alphabetic()).map(|&c| ksh93_pat_swapcase(c)));
                        out.push(orig[close]);
                        i = close + 1;
                    }
                    None => {
                        out.push(orig[i]);
                        i += 1;
                    }
                }
            }
            c if c.is_ascii_alphabetic() => {
                out.extend(['[', c, ksh93_pat_swapcase(c), ']']);
                i += 1;
            }
            _ => {
                out.push(orig[i]);
                i += 1;
            }
        }
    }
}

/// Translate a POSIX basic regular expression to the ERE the `regex` crate
/// reads: `\( \) \{ \} \| \+ \?` become operators, their bare forms literals.
fn ksh93_bre_to_ere(bre: &[char]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < bre.len() {
        match (bre[i], bre.get(i + 1)) {
            ('\\', Some(&n)) if "(){}|+?".contains(n) => {
                out.push(n);
                i += 2;
            }
            ('\\', Some(&n)) => {
                out.extend(['\\', n]);
                i += 2;
            }
            (c, _) if "(){}|+?".contains(c) => {
                out.extend(['\\', c]);
                i += 1;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// ksh93 pattern options: `[[ ab == ~(i)AB ]]`, `~(i:AB)c`, `a~(i)B*`,
/// `~(E)b+`, `~(F)a.c`, `~(G)a\(b\)c`. `Some(verdict)` when `pat` carries a
/// `~(options)` prefix this port models, `None` to let the ordinary matcher
/// run. Verified against ksh93u+m 1.0.10:
///
///   * `i` makes the pattern text after it case-insensitive until `~(-i)`; the
///     scoped `~(i:pat)` form covers only `pat`.
///   * `F`, `E`/`P` and `G` replace the glob with a fixed string / ERE / BRE
///     that is SEARCHED for anywhere in the subject (`[[ abc == ~(E)b ]]` is
///     true), unlike a glob, which must cover it.
///   * `K`, `X`, `N` read as the default ksh glob.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart; --ksh only (mksh has no
/// `~(…)` pattern options). In zsh `~(…)` is extended-glob negation. !!!
pub fn ksh93_flagged_match(s: &str, pat: &str) -> Option<bool> {
    use crate::ported::zsh_h::{Inpar, Outpar, Tilde};
    if !crate::dash_mode::ksh93_mode() {
        return None;
    }
    let orig: Vec<char> = pat.chars().collect();
    let cs: Vec<char> = orig
        .iter()
        .map(|&c| match c {
            Tilde => '~',
            Inpar => '(',
            Outpar => ')',
            c => c,
        })
        .collect();
    let first = (0..cs.len().saturating_sub(1)).find(|&i| {
        cs[i] == '~' && cs[i + 1] == '(' && (i == 0 || cs[i - 1] != '\\')
    })?;
    let (icase, engine) = {
        let end = cs[first + 2..].iter().position(|&c| !(c.is_ascii_alphabetic() || c == '-' || c == '+'))? + first + 2;
        ksh93_pat_options(&cs[first + 2..end])?
    };
    if engine != Ksh93PatEngine::Glob {
        // The regex / fixed-string engines take the whole pattern: no text
        // around the `~(options)` group is modelled.
        let end = cs[first + 2..].iter().position(|&c| !(c.is_ascii_alphabetic() || c == '-' || c == '+'))? + first + 2;
        let body: Vec<char> = match cs[end] {
            ')' if first == 0 => cs[end + 1..].to_vec(),
            ':' if first == 0 && ksh93_pat_group_end(&cs, 1) == Some(cs.len()) => cs[end + 1..cs.len() - 1].to_vec(),
            _ => return None,
        };
        let text: String = body.iter().collect();
        return match engine {
            Ksh93PatEngine::Fixed if icase => Some(s.to_lowercase().contains(&text.to_lowercase())),
            Ksh93PatEngine::Fixed => Some(s.contains(&text)),
            _ => {
                let re = if engine == Ksh93PatEngine::Bre { ksh93_bre_to_ere(&body) } else { text };
                regex::RegexBuilder::new(&re).case_insensitive(icase).build().ok().map(|r| r.is_match(s))
            }
        };
    }
    // Glob engine: walk the pattern once, tracking the case-fold state.
    let mut out = String::new();
    let mut fold = false;
    let mut i = 0;
    let mut lit_from = 0;
    while i < cs.len() {
        if cs[i] == '\\' {
            i += 2;
            continue;
        }
        if cs[i] == '~' && cs.get(i + 1) == Some(&'(') {
            let Some(rel) = cs[i + 2..].iter().position(|&c| !(c.is_ascii_alphabetic() || c == '-' || c == '+')) else { break };
            let end = i + 2 + rel;
            let Some((ic, eng)) = ksh93_pat_options(&cs[i + 2..end]) else { return None };
            if eng != Ksh93PatEngine::Glob || !matches!(cs[end], ')' | ':') {
                return None;
            }
            flush_range(&mut out, &orig, &cs, lit_from, i, fold);
            // `~(-i)` clears the fold; `~(i)` sets it; other letters leave it.
            let off = cs[i + 2..end].contains(&'-');
            let new_fold = if cs[i + 2..end].contains(&'i') { !off && ic } else { fold };
            if cs[end] == ')' {
                fold = new_fold;
                i = end + 1;
            } else {
                let close = ksh93_pat_group_end(&cs, i + 1)?;
                flush_range(&mut out, &orig, &cs, end + 1, close - 1, new_fold);
                i = close;
            }
            lit_from = i;
            continue;
        }
        i += 1;
    }
    flush_range(&mut out, &orig, &cs, lit_from, cs.len(), fold);
    Some(crate::vm_helper::glob_match_static(s, &out))
}

/// [`ksh93_pat_fold_case`] over `orig[from..to]` when `fold`, else a copy.
fn flush_range(out: &mut String, orig: &[char], cs: &[char], from: usize, to: usize, fold: bool) {
    if fold {
        ksh93_pat_fold_case(&orig[from..to], &cs[from..to], out);
    } else {
        out.extend(&orig[from..to]);
    }
}

/// `s[from..to)` (char indices) with every glob metacharacter and backslash
/// backslash-escaped; returns the new text and how many characters were added
/// before `to`. See `PAT_OPERAND_ESCAPE_QUOTED` in `ported/subst.rs`.
///
/// !!! RUST-ONLY HELPER !!! — no C counterpart.
pub fn escape_quoted_splice(s: &str, from: usize, to: usize) -> (String, usize) {
    let mut out = String::with_capacity(s.len() + 8);
    let mut added = 0usize;
    for (i, c) in s.chars().enumerate() {
        if i >= from
            && i < to
            && matches!(c, '\\' | '*' | '?' | '[' | ']' | '(' | ')' | '|' | '<' | '>' | '^' | '#' | '~' | '!' | '-')
        {
            out.push('\\');
            added += 1;
        }
        out.push(c);
    }
    (out, added)
}
