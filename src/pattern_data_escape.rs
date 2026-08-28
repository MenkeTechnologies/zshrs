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
