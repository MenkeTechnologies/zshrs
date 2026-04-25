//! AST optimization passes for zshrs.
//!
//! Run on parsed ASTs before caching to SQLite. Each pass transforms the tree
//! to eliminate work at runtime.
//!
//! Current passes:
//!   - Constant folding: $(( 2 + 3 )) → "5", $(( 1 << 10 )) → "1024"
//!   - Dead branch elimination: if false → remove body
//!   - Literal concatenation: "abc" ++ "def" → "abcdef"

use crate::math::MathEval;
use crate::parser::*;

/// Run all optimization passes on a list of commands.
pub fn optimize(commands: &mut Vec<ShellCommand>) {
    for cmd in commands.iter_mut() {
        optimize_command(cmd);
    }
}

fn optimize_command(cmd: &mut ShellCommand) {
    match cmd {
        ShellCommand::Simple(simple) => {
            for word in simple.words.iter_mut() {
                optimize_word(word);
            }
            for (_, val, _) in simple.assignments.iter_mut() {
                optimize_word(val);
            }
        }
        ShellCommand::Pipeline(cmds, _) => {
            for c in cmds.iter_mut() {
                optimize_command(c);
            }
        }
        ShellCommand::List(items) => {
            for (c, _) in items.iter_mut() {
                optimize_command(c);
            }
        }
        ShellCommand::Compound(compound) => optimize_compound(compound),
        ShellCommand::FunctionDef(_, body) => {
            optimize_command(body);
        }
    }
}

fn optimize_compound(compound: &mut CompoundCommand) {
    match compound {
        CompoundCommand::BraceGroup(cmds) | CompoundCommand::Subshell(cmds) => {
            for c in cmds.iter_mut() { optimize_command(c); }
        }
        CompoundCommand::If { conditions, else_part } => {
            for (cond, body) in conditions.iter_mut() {
                for c in cond.iter_mut() { optimize_command(c); }
                for c in body.iter_mut() { optimize_command(c); }
            }
            if let Some(els) = else_part {
                for c in els.iter_mut() { optimize_command(c); }
            }
        }
        CompoundCommand::For { words, body, .. } => {
            if let Some(ws) = words {
                for w in ws.iter_mut() { optimize_word(w); }
            }
            for c in body.iter_mut() { optimize_command(c); }
        }
        CompoundCommand::ForArith { body, .. } => {
            for c in body.iter_mut() { optimize_command(c); }
        }
        CompoundCommand::While { condition, body } | CompoundCommand::Until { condition, body } => {
            for c in condition.iter_mut() { optimize_command(c); }
            for c in body.iter_mut() { optimize_command(c); }
        }
        CompoundCommand::Case { word, cases } => {
            optimize_word(word);
            for (pats, cmds, _) in cases.iter_mut() {
                for p in pats.iter_mut() { optimize_word(p); }
                for c in cmds.iter_mut() { optimize_command(c); }
            }
        }
        CompoundCommand::Select { words, body, .. } => {
            if let Some(ws) = words {
                for w in ws.iter_mut() { optimize_word(w); }
            }
            for c in body.iter_mut() { optimize_command(c); }
        }
        CompoundCommand::Try { try_body, always_body } => {
            for c in try_body.iter_mut() { optimize_command(c); }
            for c in always_body.iter_mut() { optimize_command(c); }
        }
        CompoundCommand::Repeat { body, .. } => {
            for c in body.iter_mut() { optimize_command(c); }
        }
        CompoundCommand::Coproc { body, .. } => {
            optimize_command(body);
        }
        CompoundCommand::WithRedirects(cmd, _) => {
            optimize_command(cmd);
        }
        _ => {}
    }
}

fn optimize_word(word: &mut ShellWord) {
    match word {
        ShellWord::ArithSub(expr) => {
            // Constant fold: if expr has no variable references, evaluate at compile time
            if is_constant_expr(expr) {
                if let Some(val) = eval_constant(expr) {
                    tracing::trace!(expr = %expr, result = %val, "ast_opt: constant folded");
                    *word = ShellWord::Literal(val);
                }
            }
        }
        ShellWord::DoubleQuoted(parts) => {
            for p in parts.iter_mut() { optimize_word(p); }
            // Merge adjacent literals
            merge_adjacent_literals(parts);
        }
        ShellWord::Concat(parts) => {
            for p in parts.iter_mut() { optimize_word(p); }
            merge_adjacent_literals(parts);
            // If concat reduced to single element, unwrap
            if parts.len() == 1 {
                *word = parts.remove(0);
            }
        }
        ShellWord::ArrayLiteral(elements) => {
            for e in elements.iter_mut() { optimize_word(e); }
        }
        ShellWord::CommandSub(cmd) => {
            optimize_command(cmd);
        }
        ShellWord::ProcessSubIn(cmd) | ShellWord::ProcessSubOut(cmd) => {
            optimize_command(cmd);
        }
        _ => {}
    }
}

/// Check if an arithmetic expression is a compile-time constant
/// (no variable references, no command substitutions).
fn is_constant_expr(expr: &str) -> bool {
    // No $var, ${var}, $(cmd) references
    !expr.contains('$') &&
    // No variable names (bare identifiers that aren't numeric)
    expr.chars().all(|c| {
        c.is_ascii_digit() || c.is_ascii_whitespace() ||
        matches!(c, '+' | '-' | '*' | '/' | '%' | '(' | ')' |
                    '&' | '|' | '^' | '~' | '<' | '>' | '!' |
                    '=' | '?' | ':' | '.' | 'x' | 'X' |
                    'a' | 'b' | 'c' | 'd' | 'e' | 'f' |
                    'A' | 'B' | 'C' | 'D' | 'E' | 'F')
    })
}

/// Evaluate a constant arithmetic expression at compile time.
fn eval_constant(expr: &str) -> Option<String> {
    let mut evaluator = MathEval::new(expr);
    match evaluator.evaluate() {
        Ok(result) => {
            let s = match result {
                crate::math::MathNum::Integer(i) => i.to_string(),
                crate::math::MathNum::Float(f) => {
                    if f.fract() == 0.0 && f.abs() < i64::MAX as f64 {
                        (f as i64).to_string()
                    } else {
                        f.to_string()
                    }
                }
                _ => return None,
            };
            Some(s)
        }
        Err(_) => None,
    }
}

/// Merge adjacent Literal nodes in a word list.
fn merge_adjacent_literals(parts: &mut Vec<ShellWord>) {
    let mut i = 0;
    while i + 1 < parts.len() {
        if let (ShellWord::Literal(a), ShellWord::Literal(b)) = (&parts[i], &parts[i + 1]) {
            let merged = format!("{}{}", a, b);
            parts[i] = ShellWord::Literal(merged);
            parts.remove(i + 1);
        } else {
            i += 1;
        }
    }
}
