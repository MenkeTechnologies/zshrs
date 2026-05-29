//! Non-C helper: format raw function-body source the way zsh `typeset -f`
//! presents multi-statement bodies (split on top-level `;` / newlines).

/// Format a function body for `functions` / `which -x` style output.
pub struct FuncBodyFmt;

impl FuncBodyFmt {
    /// `render` — see implementation.
    pub fn render(body: &str) -> String {
        let mut lines: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut depth_paren = 0i32;
        let mut depth_brace = 0i32;
        let mut in_squote = false;
        let mut in_dquote = false;
        let mut prev = '\0';
        for c in body.chars() {
            let escaped = prev == '\\';
            match c {
                '\'' if !in_dquote && !escaped => {
                    in_squote = !in_squote;
                    current.push(c);
                }
                '"' if !in_squote && !escaped => {
                    in_dquote = !in_dquote;
                    current.push(c);
                }
                '(' | '[' | '{' if !in_squote && !in_dquote => {
                    if c == '{' {
                        depth_brace += 1;
                    } else {
                        depth_paren += 1;
                    }
                    current.push(c);
                }
                ')' | ']' | '}' if !in_squote && !in_dquote => {
                    if c == '}' {
                        depth_brace -= 1;
                    } else {
                        depth_paren -= 1;
                    }
                    current.push(c);
                }
                ';' | '\n' if !in_squote && !in_dquote && depth_paren == 0 && depth_brace == 0 => {
                    let t = current.trim().to_string();
                    if !t.is_empty() {
                        lines.push(t);
                    }
                    current.clear();
                }
                _ => current.push(c),
            }
            prev = c;
        }
        let t = current.trim().to_string();
        if !t.is_empty() {
            lines.push(t);
        }
        lines.join("\n\t")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body_renders_empty() {
        assert_eq!(FuncBodyFmt::render(""), "");
    }

    #[test]
    fn whitespace_only_body_renders_empty() {
        assert_eq!(FuncBodyFmt::render("   \n  \t  "), "");
    }

    #[test]
    fn single_statement_renders_one_line() {
        assert_eq!(FuncBodyFmt::render("echo hi"), "echo hi");
    }

    #[test]
    fn semicolon_splits_top_level() {
        assert_eq!(FuncBodyFmt::render("echo a; echo b"), "echo a\n\techo b");
    }

    #[test]
    fn newline_splits_top_level() {
        assert_eq!(FuncBodyFmt::render("echo a\necho b"), "echo a\n\techo b");
    }

    #[test]
    fn empty_segments_are_dropped() {
        // `;;` between two real statements should collapse cleanly.
        assert_eq!(FuncBodyFmt::render("echo a;;echo b"), "echo a\n\techo b");
        assert_eq!(FuncBodyFmt::render(";echo a;"), "echo a");
    }

    #[test]
    fn segments_are_trimmed() {
        assert_eq!(
            FuncBodyFmt::render("  echo a  ;  echo b  "),
            "echo a\n\techo b"
        );
    }

    #[test]
    fn semicolon_in_single_quotes_does_not_split() {
        // `;` inside '…' is literal — must stay attached.
        let out = FuncBodyFmt::render("echo 'a;b'; echo c");
        assert_eq!(out, "echo 'a;b'\n\techo c");
    }

    #[test]
    fn semicolon_in_double_quotes_does_not_split() {
        let out = FuncBodyFmt::render("echo \"a;b\"; echo c");
        assert_eq!(out, "echo \"a;b\"\n\techo c");
    }

    #[test]
    fn newline_in_double_quotes_does_not_split() {
        let out = FuncBodyFmt::render("echo \"a\nb\"; echo c");
        assert_eq!(out, "echo \"a\nb\"\n\techo c");
    }

    #[test]
    fn semicolon_in_brace_group_is_kept_intact() {
        // `{ … ; … }` is one statement at the top level.
        let out = FuncBodyFmt::render("{ echo a; echo b }; echo c");
        assert_eq!(out, "{ echo a; echo b }\n\techo c");
    }

    #[test]
    fn semicolon_in_paren_subshell_is_kept_intact() {
        let out = FuncBodyFmt::render("(echo a; echo b); echo c");
        assert_eq!(out, "(echo a; echo b)\n\techo c");
    }

    #[test]
    fn nested_braces_balance_correctly() {
        let out = FuncBodyFmt::render("{ { a; b }; c }; d");
        assert_eq!(out, "{ { a; b }; c }\n\td");
    }

    #[test]
    fn dollar_paren_command_substitution_is_kept() {
        // `;` inside $( … ) must not split the outer statement.
        let out = FuncBodyFmt::render("x=$(a; b); echo $x");
        assert_eq!(out, "x=$(a; b)\n\techo $x");
    }

    #[test]
    fn double_quote_inside_single_quote_is_literal() {
        // Inside '…' the " is a plain char and must NOT open dquote mode.
        // So the trailing `;` outside the quotes still splits.
        let out = FuncBodyFmt::render("echo 'a\"b'; echo c");
        assert_eq!(out, "echo 'a\"b'\n\techo c");
    }

    #[test]
    fn single_quote_inside_double_quote_is_literal() {
        let out = FuncBodyFmt::render("echo \"a'b\"; echo c");
        assert_eq!(out, "echo \"a'b\"\n\techo c");
    }

    #[test]
    fn three_statements_split_correctly() {
        assert_eq!(FuncBodyFmt::render("a;b;c"), "a\n\tb\n\tc");
    }

    #[test]
    fn mixed_semicolon_and_newline() {
        assert_eq!(FuncBodyFmt::render("a;b\nc;d"), "a\n\tb\n\tc\n\td");
    }
}
