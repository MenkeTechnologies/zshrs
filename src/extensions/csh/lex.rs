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
            (_, '\\') => match chars.next() {
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
            _ => cur.push(c),
        }
    }
    push_line(&mut out, &mut cur);
    out
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
