//! Non-C helper: format raw function-body source the way zsh `typeset -f`
//! presents multi-statement bodies (split on top-level `;` / newlines).

/// Format a function body for `functions` / `which -x` style output.
pub struct FuncBodyFmt;

impl FuncBodyFmt {
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
                ';' | '\n'
                    if !in_squote && !in_dquote && depth_paren == 0 && depth_brace == 0 =>
                {
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
