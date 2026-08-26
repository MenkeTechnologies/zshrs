//! The TOKENIZED twin of the word being completed.
//!
//! NOT a port of any C function — this is Rust-original glue that makes up
//! for a shape difference between C's completion pipeline and this port's.
//!
//! In C the word travels TOKENIZED from `get_comp_string`
//! (`Src/Zle/zle_tricky.c:2221`) through `docompletion` (c:851) and
//! `makecomplist` (`Src/Zle/compcore.c:952`) into `callcompfunc`, and only
//! gets untokenized at the very end, INSIDE `callcompfunc`, after
//! `multiquote(s, 0)` has run over it (compcore.c:700-703/711-717). That
//! ordering is load-bearing: `quotestring` passes a parser token straight
//! through (`Src/utils.c:6392`), so a live glob `*` (the token `Star`) stays
//! bare while a `*` the user escaped — a plain `*` in the word by then —
//! gets its backslash back. Run the same `multiquote` over the UNTOKENIZED
//! word and it escapes the live metacharacters instead, publishing `\*` for
//! `ls *`.
//!
//! This port untokenizes early, at `get_comp_string`'s return
//! (`zle_tricky.rs`, `return Some(untokenize(&s))`), and stashes the
//! tokenized form in `zle_tricky::COMP_STRING_TOK`. The twin therefore has to
//! be carried forward by hand — narrowed wherever `makecomplist` narrows the
//! word (c:952-956) — which is what this module holds.
//!
//! Every accessor is fallible on purpose: `None` means "no usable twin" and
//! each caller then falls back to the untokenized word it already has.

use crate::ported::lex::untokenize;
use crate::ported::zsh_h::{Qstring, Snull, Stringg};
use std::sync::{Mutex, OnceLock};

/// The narrowed tokenized twin of the word `callcompfunc` publishes.
static COMPS_TOK: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<String>> {
    COMPS_TOK.get_or_init(|| Mutex::new(None))
}

/// Publish the twin for the completion about to run.
pub fn set(tok: Option<String>) {
    if let Ok(mut g) = cell().lock() {
        *g = tok;
    }
}

/// Read the twin published by [`set`].
pub fn get() -> Option<String> {
    cell().lock().ok().and_then(|g| g.clone())
}

/// Map a byte span of `untokenize(tok)` back onto `tok` itself.
///
/// `untokenize` (`lex.rs`) emits either nothing (`Snull`/`Dnull`/`Nularg`) or
/// exactly one char per input char, so the mapping is positional. The one
/// exception is a `$'…'` region, which it decodes as a unit; return `None`
/// there rather than guess. The result is self-checked: it must untokenize
/// back to exactly the span that was asked for.
pub fn span(tok: &str, ubeg: usize, ulen: usize) -> Option<String> {
    let uwhole = untokenize(tok);
    let uend = ubeg.checked_add(ulen)?;
    if uend > uwhole.len() || !uwhole.is_char_boundary(ubeg) || !uwhole.is_char_boundary(uend) {
        return None;
    }
    let want = &uwhole[ubeg..uend];

    let chars: Vec<(usize, char)> = tok.char_indices().collect();
    let mut upos = 0usize;
    let mut beg: Option<usize> = None;
    let mut end = tok.len();
    for (n, &(bi, c)) in chars.iter().enumerate() {
        if (c == Stringg || c == Qstring) && chars.get(n + 1).map(|x| x.1) == Some(Snull) {
            return None;
        }
        if beg.is_none() && upos >= ubeg {
            beg = Some(bi);
        }
        if upos >= uend {
            end = bi;
            break;
        }
        upos += untokenize(&c.to_string()).len();
    }
    if beg.is_none() && upos >= ubeg {
        beg = Some(tok.len());
    }
    let cand = tok.get(beg?..end)?.to_string();
    if untokenize(&cand) == want {
        Some(cand)
    } else {
        None
    }
}

/// Byte index in `tok` that corresponds to byte index `ubyte` of
/// `untokenize(tok)`.
///
/// `check_param` (`Src/Zle/compcore.c:1113`) indexes its argument with the
/// global `offs`, and in C that argument is the TOKENIZED word — so a port
/// that hands it the tokenized twin has to hand it a tokenized `offs` too.
/// Same positional mapping `span` relies on, and the same `$'…'` bail-out:
/// `untokenize` decodes that region as a unit, so no per-char index exists.
/// `None` also when `ubyte` lands inside a char rather than on its boundary.
pub fn tok_index(tok: &str, ubyte: usize) -> Option<usize> {
    let chars: Vec<(usize, char)> = tok.char_indices().collect();
    let mut upos = 0usize;
    for (n, &(bi, c)) in chars.iter().enumerate() {
        let ulen = untokenize(&c.to_string()).len();
        // A marker `untokenize` drops (`Snull`/`Dnull`/`Nularg`) occupies no
        // untokenized byte, so it can never BE the answer — skip past it and
        // land on the char that actually renders at `ubyte`.
        if ulen == 0 {
            continue;
        }
        if upos == ubyte {
            return Some(bi);
        }
        if (c == Stringg || c == Qstring) && chars.get(n + 1).map(|x| x.1) == Some(Snull) {
            return None;
        }
        upos += ulen;
        if upos > ubyte {
            return None;
        }
    }
    if upos == ubyte {
        Some(tok.len())
    } else {
        None
    }
}

/// The inverse of [`tok_index`]: byte index in `untokenize(tok)` for byte
/// index `tbyte` of `tok`. `None` when `tbyte` is not a char boundary.
pub fn untok_index(tok: &str, tbyte: usize) -> Option<usize> {
    tok.get(..tbyte).map(|head| untokenize(head).len())
}

/// The `s`-side effect of the quote-marker cleanup loop at
/// `Src/Zle/zle_tricky.c:1788-1926`.
///
/// That loop does two separate jobs: it deletes the quote characters from the
/// LINE (`foredel`, adjusting `zlemetacs`/`we`/`offs`), and it `chuck`s the
/// matching `inull` markers out of the word `s` (c:1919-1921, "we need to get
/// rid of all the quotation bits"). Only the second is what `multiquote` at
/// c:700 depends on, and it is the only one replayed here: `get_comp_string`
/// itself now ports the whole loop (`zle_tricky.rs`, the `c:1787-1926` block),
/// so replaying `foredel` would delete the quotes from the line twice.
///
/// Because that block runs BEFORE the tokenized word is stashed in
/// `zle_tricky::COMP_STRING_TOK`, the twin normally arrives here with its
/// markers already chucked and this walk is a no-op — it is idempotent by
/// construction and stays as the guard for twins that did not come through
/// `get_comp_string`.
///
/// A `$'…'` region is left intact. C substitutes its decoded value into `s`
/// there (c:1805-1864); this port defers that to `untokenize`.
pub fn strip_nulls(tok: &str) -> String {
    use crate::ported::ztype_h::inull;

    let chars: Vec<char> = tok.chars().collect();
    let mut out = String::with_capacity(tok.len());
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        // c:1790 / c:1879 — `$'…'`: copy the region through untouched.
        if (c == Stringg || c == Qstring) && chars.get(i + 1) == Some(&Snull) {
            out.push(c);
            out.push(Snull);
            i += 2;
            while i < chars.len() {
                let d = chars[i];
                out.push(d);
                i += 1;
                if d == Snull {
                    break;
                }
            }
            continue;
        }
        // c:1881-1882 — `else if (inull(*p)) skipchars = 1;`, chucked at
        // c:1920-1921.
        let b = if (c as u32) < 0x100 { c as u32 as u8 } else { 0 };
        if inull(b) {
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// `multiquote(s, 0)` over a tokenized twin, then `untokenize` — exactly
/// `Src/Zle/compcore.c:700`+`701` (and `711`+`712` / `715`+`716`).
///
/// `None` in, `None` out: the caller keeps the untokenized word it has.
pub fn multiquoted(tok: Option<&str>) -> Option<String> {
    tok.map(|t| untokenize(&crate::ported::zle::compcore::multiquote(&strip_nulls(t), 0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::Dnull;

    #[test]
    fn span_maps_past_dropped_quote_markers() {
        // `"$PA` as the lexer leaves it: Dnull + Qstring + "PA". `untokenize`
        // drops the Dnull and renders Qstring as `$`, so `PA` sits at byte 2
        // of `"$PA`… no: of `$PA`.
        let tok = format!("{Dnull}{Qstring}PA");
        assert_eq!(untokenize(&tok), "$PA");
        assert_eq!(span(&tok, 1, 2).as_deref(), Some("PA"));
        assert_eq!(span(&tok, 0, 3), Some(tok.clone()));
    }

    /// `check_param` indexes its argument with the global `offs`, so feeding
    /// it the tokenized twin means feeding it a tokenized `offs` too: every
    /// token char is 2 bytes here against the 1 byte it untokenizes to, and a
    /// dropped `Dnull` costs a byte outright.
    #[test]
    fn tok_index_and_untok_index_are_inverses() {
        let tok = format!("{Dnull}{Qstring}PA");
        assert_eq!(untokenize(&tok), "$PA");
        // untokenized byte 0 (`$`) is the Qstring, which follows the Dnull.
        assert_eq!(tok_index(&tok, 0), Some(Dnull.len_utf8()));
        assert_eq!(
            tok_index(&tok, 1),
            Some(Dnull.len_utf8() + Qstring.len_utf8())
        );
        assert_eq!(tok_index(&tok, 3), Some(tok.len()));
        assert_eq!(tok_index(&tok, 4), None);
        for u in 0..=3 {
            let t = tok_index(&tok, u).unwrap();
            assert_eq!(untok_index(&tok, t), Some(u));
        }
    }

    #[test]
    fn tok_index_refuses_a_dollar_quote_region() {
        let tok = format!("{Stringg}{Snull}a{Snull}");
        assert_eq!(tok_index(&tok, 1), None);
    }

    #[test]
    fn span_refuses_a_dollar_quote_region() {
        // `$'a'` decodes as a unit, so the mapping is not positional.
        let tok = format!("{Stringg}{Snull}a{Snull}");
        assert_eq!(span(&tok, 0, 1), None);
    }

    #[test]
    fn strip_nulls_chucks_quote_markers_but_keeps_dollar_quote() {
        // `inull` is typtab-backed (ztype_h.rs, `zistype(x, INULL)`), and a
        // test binary never runs `setupvals`, so the table has to be seeded.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(strip_nulls(&format!("{Dnull}a b")), "a b");
        let dq = format!("{Stringg}{Snull}a{Snull}");
        assert_eq!(strip_nulls(&dq), dq);
    }
}
