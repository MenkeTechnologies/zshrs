//! Quote-aware line handling shared by every translator layer.

/// Split `src` into logical lines: a backslash-newline joins two physical
/// lines with a blank (tcsh: `a\<nl>b` echoes `a b`); a `#` outside quotes starts a comment that runs to end of line, except
/// in the `$#x` and `${#x}` forms;
/// blank lines are dropped. Quotes (`'`, `"`, `` ` ``) and backslash escapes
/// are honoured so `echo '#x'` keeps its `#`.
pub fn logical_lines(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            // `\"` inside "…" is not an escape in csh: it ends the string
            (Some('"'), '\\') if chars.peek() == Some(&'"') => cur.push('\\'),
            (_, '\\') => match chars.next() {
                // inside "…" the newline stays (the word translator drops
                // the backslash); elsewhere it is a blank
                Some('\n') if quote == Some('"') => {
                    cur.push('\\');
                    cur.push('\n');
                }
                Some('\n') => cur.push(' '),
                Some(n) => {
                    cur.push('\\');
                    cur.push(n);
                }
                None => cur.push('\\'),
            },
            (None, '\'' | '"' | '`') => {
                quote = Some(c);
                cur.push(c);
            }
            (Some(q), _) if c == q => {
                quote = None;
                cur.push(c);
            }
            (None, '#') if cur.ends_with('$') || cur.ends_with("${") => cur.push(c),
            (None, '#') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
                push_line(&mut out, &mut cur);
            }
            (None, '\n') => push_line(&mut out, &mut cur),
            // a quote does not span lines (only `\<newline>` inside "…"
            // continues one): the line ends and its quote stays unmatched
            (Some(_), '\n') => {
                quote = None;
                push_line(&mut out, &mut cur);
            }
            _ => cur.push(c),
        }
    }
    push_line(&mut out, &mut cur);
    out
}

/// Marks a logical line that is a here-document body line (or its
/// terminator): literal text the translator must pass through untouched.
pub const HEREDOC_RAW: char = '\u{1}';

/// The here-document delimiter a physical line opens, if any: `<<WORD`,
/// `<< WORD`, `<<'WORD'`, `<<"WORD"` or `<<\WORD` outside quotes and
/// parentheses. An `@` or `set` line's `<<` is the shift operator.
fn heredoc_delim(line: &str) -> Option<String> {
    let t = line.trim_start();
    if t.starts_with('@') || t.starts_with("set ") {
        return None;
    }
    let mut quote: Option<char> = None;
    let mut depth = 0i32;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match (quote, c) {
            (_, '\\') => i += 1,
            (None, '\'' | '"' | '`') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, '<') if depth == 0 && chars.get(i + 1) == Some(&'<') => {
                let mut j = i + 2;
                while matches!(chars.get(j), Some(' ' | '\t')) {
                    j += 1;
                }
                let mut word = String::new();
                let mut q: Option<char> = None;
                while let Some(&w) = chars.get(j) {
                    // tcsh compares a terminator line with the word as
                    // typed, quotes and backslashes included.
                    match (q, w) {
                        (None, '\'' | '"') => q = Some(w),
                        (Some(x), _) if w == x => q = None,
                        (None, ' ' | '\t' | ';' | '|' | '&' | '<' | '>' | ')') => break,
                        _ => {}
                    }
                    word.push(w);
                    j += 1;
                }
                return (!word.is_empty()).then_some(word);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// [`logical_lines`] for a whole script: here-document bodies (and their
/// terminators) are kept verbatim as `HEREDOC_RAW`-prefixed lines — no
/// comment stripping, no continuation joining, blank lines kept.
pub fn script_lines(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chunk = String::new();
    // the newline ending the last line does not start another one
    let mut physical = src.strip_suffix('\n').unwrap_or(src).split('\n');
    while let Some(line) = physical.next() {
        chunk.push_str(line);
        chunk.push('\n');
        if line.ends_with('\\') {
            continue;
        }
        let Some(delim) = heredoc_delim(line) else {
            continue;
        };
        out.extend(logical_lines(&chunk));
        chunk.clear();
        let quoted = delim.contains(['\'', '"', '\\']);
        for body in physical.by_ref() {
            // zsh joins a line ending in a lone backslash with the next one;
            // csh keeps both lines. A doubled backslash prints as one.
            let odd_tail = body.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1;
            let extra = if odd_tail && !quoted { "\\" } else { "" };
            out.push(format!("{HEREDOC_RAW}{body}{extra}"));
            if body == delim {
                break;
            }
        }
    }
    out.extend(logical_lines(&chunk));
    out
}

/// For the text after a `:` in a variable reference: when it is a
/// substitution modifier (`s/l/r/`, `as/l/r/`, `gs/l/r/`) with both closing
/// delimiters present, the index of the last char of it. Its operands may
/// hold blanks that must not split the word; a missing closer returns `None`
/// and leaves the blank as a word break.
pub fn subst_modifier_end(all: &[char], start: usize) -> Option<usize> {
    let mut j = start;
    while matches!(all.get(j), Some('a' | 'g' | 'G')) {
        j += 1;
    }
    if all.get(j) != Some(&'s') {
        return None;
    }
    let d = *all
        .get(j + 1)
        .filter(|d| !d.is_alphanumeric() && !d.is_whitespace())?;
    all[j + 2..]
        .iter()
        .enumerate()
        .filter(|(_, &x)| x == d)
        .map(|(k, _)| j + 2 + k)
        .nth(1)
}

fn push_line(out: &mut Vec<String>, cur: &mut String) {
    let line = cur.trim();
    if !line.is_empty() {
        out.push(line.to_string());
    }
    cur.clear();
}

/// Split `line` on every top-level occurrence of `sep` — outside quotes,
/// backslash escapes and parentheses. Pieces are returned untrimmed.
pub fn split_unquoted(line: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut depth = 0i32;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (_, '\\') => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (None, '\'' | '"' | '`') => {
                quote = Some(c);
                cur.push(c);
            }
            (Some(q), _) if c == q => {
                quote = None;
                cur.push(c);
            }
            (None, '(') => {
                depth += 1;
                cur.push(c);
            }
            (None, ')') => {
                depth -= 1;
                cur.push(c);
            }
            (None, _) if c == sep && depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Split a command line into csh words: whitespace-separated, quotes and
/// backslashes kept verbatim inside each word, parenthesised groups kept
/// whole. `(a b c)` is one word.
pub fn split_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut depth = 0i32;
    let all: Vec<char> = line.chars().collect();
    let mut pos = 0;
    while pos < all.len() {
        let c = all[pos];
        pos += 1;
        match (quote, c) {
            (_, '\\') => {
                cur.push(c);
                if let Some(&n) = all.get(pos) {
                    cur.push(n);
                    pos += 1;
                }
            }
            // `$v:s/a/b c/` — the operands of a variable's substitution
            // modifier may hold blanks; they belong to the word.
            (None, ':') if cur.contains('$') => {
                cur.push(c);
                if let Some(end) = subst_modifier_end(&all, pos) {
                    cur.extend(&all[pos..=end]);
                    pos = end + 1;
                }
            }
            (None, '\'' | '"' | '`') => {
                quote = Some(c);
                cur.push(c);
            }
            (Some(q), _) if c == q => {
                quote = None;
                cur.push(c);
            }
            (None, '(') => {
                depth += 1;
                cur.push(c);
            }
            (None, ')') => {
                depth -= 1;
                cur.push(c);
            }
            (None, _) if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_continuations_and_strips_comments_outside_quotes() {
        let got = logical_lines("echo a\\\nb # c\n\necho '#x'\n");
        assert_eq!(got, vec!["echo a b", "echo '#x'"]);
    }

    #[test]
    fn hash_after_dollar_is_a_count_not_a_comment() {
        assert_eq!(logical_lines("echo $#a ${#b} # c"), vec!["echo $#a ${#b}"]);
    }

    #[test]
    fn split_respects_quotes_and_parens() {
        assert_eq!(split_unquoted("a; b ';' c; (d; e)", ';'), vec!["a", " b ';' c", " (d; e)"]);
    }

    #[test]
    fn words_keep_groups_whole() {
        assert_eq!(split_words("set a = (1 2 3)"), vec!["set", "a", "=", "(1 2 3)"]);
    }
}
