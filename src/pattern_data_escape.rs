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
