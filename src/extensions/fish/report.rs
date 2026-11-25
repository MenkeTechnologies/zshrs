//! fish's located error messages, for the `--fish` drop-in.
//!
//! !!! RUST-ONLY: zsh has no counterpart. The spec is fish's own code
//! (vendor/fish): `Parser::get_backtrace` and `stack_trace` (parser.rs),
//! `ParseError::describe_with_prefix` (parse_constants.rs) and the block
//! descriptions of `append_block_description_to_stack_trace`.
//!
//! An error names where it happened: `fish: ` or `FILE (line N): ` and the text,
//! then the source line with a caret under the offending part, then one entry
//! per function call, command substitution and sourced file it happened in.
//! The translated code keeps what that needs at run time:
//! - `__fish_bt`: the blocks, innermost first and `␝`-separated, each
//!   `KIND␞FILE␞LINE[␞NAME[␞ARG…]]`
//!   where FILE and LINE are where the block was entered;
//! - `__fish_file`: the file the running code came from (empty for `-c`, an
//!   interactive line or `eval`), local to a function or a sourced file;
//! - `__fish_at`: `LINE COL LEN` of the command being run, set while its words
//!   expand;
//! - `__fish_srcfn` / `__fish_src`: the function, or the text (a command
//!   substitution's, an `eval`'s), the running code was parsed from.

use unicode_width::UnicodeWidthChar;

const SEP: char = '\x1e';

/// `__fish_error wildcard LINE COL LEN TEXT`, `__fish_error cmdsub STATUS LINE
/// COL LEN TEXT`, `__fish_error unknown NAME`, `__fish_error trace`.
fn run(argv: &[String]) -> i32 {
    let num = |i: usize| argv.get(i).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
    let arg = |i: usize| argv.get(i).cloned().unwrap_or_default();
    let out = match arg(1).as_str() {
        "wildcard" => {
            let (line, col, len, text) = (num(2), num(3), num(4), arg(5));
            let word: String = text.chars().skip(col).take(len).collect();
            backtrace(
                &format!("No matches for wildcard '{word}'. See `help language#wildcards-globbing`."),
                line,
                col,
                len,
                &text,
            )
        }
        "cmdsub" => {
            // expand.rs:933-977: the message for the substitution's status
            let msg = match num(2) {
                124 => "Unmatched wildcard",
                127 => "Unknown command",
                121 => "Invalid arguments",
                123 => "Commandname was invalid",
                126 => "Command not executable",
                _ => "Unknown error while evaluating command substitution",
            };
            backtrace(msg, num(3), num(4), num(5), &arg(6))
        }
        // fish_command_not_found's message (__fish_default_command_not_found_handler),
        // then the error with no text that points at the command (parse_execution.rs
        // `handle_command_not_found`)
        "unknown" => {
            let at = var("__fish_at");
            let mut at = at.split(' ').map(|n| n.parse::<usize>().unwrap_or(0));
            let (line, col, len) = (at.next().unwrap_or(0), at.next().unwrap_or(0), at.next().unwrap_or(0));
            format!(
                "fish: Unknown command: {}\n{}",
                super::complete::escape(&arg(2)),
                backtrace("", line, col, len, &source_line(line))
            )
        }
        // `status print-stack-trace`, on stdout
        "trace" => {
            print!("{}", stack_trace());
            return 0;
        }
        _ => return 2,
    };
    eprint!("{out}");
    0
}

fn var(name: &str) -> String {
    crate::ported::params::getsparam(name).unwrap_or_default()
}

/// `$assoc[key]`.
fn element(assoc: &str, key: &str) -> Option<String> {
    let keys = crate::ported::params::gethkparam(assoc)?;
    let vals = crate::ported::params::gethparam(assoc)?;
    keys.iter().zip(vals).find(|(k, _)| *k == key).map(|(_, v)| v)
}

/// The blocks in `__fish_bt`, innermost first: one string, `\x1d` between blocks
/// (a scalar costs a function call or substitution one `local` assignment).
fn frames() -> Vec<String> {
    var("__fish_bt").split('\x1d').filter(|f| !f.is_empty()).map(str::to_string).collect()
}

/// `$HOME/x` as `~/x` (`user_presentable_path`).
fn presentable(path: &str) -> String {
    let home = var("HOME");
    match path.strip_prefix(home.as_str()) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// The text of line `line` of the source the running code was parsed from.
fn source_line(line: usize) -> String {
    let nth = |src: &str, n: usize| src.split('\n').nth(n.saturating_sub(1)).unwrap_or("").to_string();
    let func = var("__fish_srcfn");
    if !func.is_empty() {
        let src = element("__fish_fnsrc", &func).unwrap_or_default();
        let first = element("__fish_fnline", &func)
            .and_then(|l| l.parse::<usize>().ok())
            .unwrap_or(1);
        return nth(&src, line + 1 - first.min(line));
    }
    let text = var("__fish_src");
    if !text.is_empty() {
        return nth(&text, line);
    }
    let file = var("__fish_file");
    if !file.is_empty() {
        return std::fs::read_to_string(&file).map(|s| nth(&s, line)).unwrap_or_default();
    }
    if crate::ported::zsh_h::interact() {
        let typed = crate::extensions::emulation_startup::FISH_MARK_CMDLINE.lock().map(|l| l.clone()).unwrap_or_default();
        return nth(&typed, line);
    }
    nth(&var("ZSH_EXECUTION_STRING"), line)
}

/// fish `get_backtrace` for an error with `text` at `line`/`col`/`len` of the
/// current source, whose line is `line_text`.
fn backtrace(text: &str, line: usize, col: usize, len: usize, line_text: &str) -> String {
    // (fish's interactive scope: a line typed at the prompt, not a function or a
    // substitution it runs)
    let interactive = crate::ported::zsh_h::interact() && frames().is_empty();
    let skip_caret = interactive && line == 1 && col == 0;
    let file = var("__fish_file");
    let prefix = if file.is_empty() { "fish".to_string() } else { format!("{} (line {line})", presentable(&file)) };
    let mut out = describe(text, &prefix, line_text, col, len, interactive, skip_caret);
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&stack_trace());
    out
}

/// fish `describe_with_prefix`: the message, then the line and a caret line
/// `^~~^` under the error.
fn describe(text: &str, prefix: &str, line_text: &str, col: usize, len: usize, interactive: bool, skip_caret: bool) -> String {
    if skip_caret && text.is_empty() {
        return String::new();
    }
    let mut out = if prefix.is_empty() { text.to_string() } else { format!("{prefix}: {text}") };
    if skip_caret || (interactive && line_text.is_empty() && col == 0) {
        return out;
    }
    let chars: Vec<char> = line_text.chars().collect();
    let col = col.min(chars.len());
    let len = len.min(chars.len() - col);
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line_text);
    out.push('\n');
    for &c in &chars[..col] {
        match c {
            '\t' => out.push('\t'),
            c => out.push_str(&" ".repeat(c.width().unwrap_or(0))),
        }
    }
    out.push('^');
    let width: usize = chars[col..col + len].iter().map(|c| c.width().unwrap_or(0)).sum();
    if len > 1 && width >= 2 {
        out.push_str(&"~".repeat(width - 2));
        out.push('^');
    }
    out
}

/// fish `stack_trace`: one entry per enclosing function call, command
/// substitution and sourced file, innermost first, each followed by where it
/// was entered when that was in a file.
fn stack_trace() -> String {
    let mut out = String::new();
    for frame in frames() {
        let mut f = frame.split(SEP);
        let (kind, file, line) = (f.next().unwrap_or(""), f.next().unwrap_or(""), f.next().unwrap_or(""));
        match kind {
            "function" => {
                out.push_str(&format!("in function '{}'", f.next().unwrap_or("")));
                let args: Vec<String> = f
                    .map(|a| if a.is_empty() { "\"\"".to_string() } else { super::complete::escape_for_insert(a) })
                    .collect();
                if !args.is_empty() {
                    out.push_str(&format!(" with arguments '{}'", args.join(" ")));
                }
                out.push('\n');
            }
            "subst" => out.push_str("in command substitution\n"),
            "source" => out.push_str(&format!("from sourcing file {}\n", presentable(f.next().unwrap_or("")))),
            _ => continue,
        }
        if !file.is_empty() {
            out.push_str(&format!("\tcalled on line {line} of file {}\n", presentable(file)));
        }
    }
    out
}

/// `__fish_frame function NAME ARG…`: the value `__fish_bt` takes in the function
/// being entered (its block, entered at `__fish_at`'s line, then the enclosing ones)
/// in `__fish_nf`,
/// and a function's own file in `__fish_nfile`, for the scope's one `local`
/// statement to take; building them here is cheaper than as words. `$?` stays as
/// it was.
fn frame(argv: &[String]) -> i32 {
    let status = crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
    let sep = SEP.to_string();
    let mut fields = vec![argv.get(1).cloned().unwrap_or_default(), var("__fish_file")];
    if argv.get(1).map(String::as_str) == Some("function") {
        let name = argv.get(2).cloned().unwrap_or_default();
        fields.push(var("__fish_at").split(' ').next().unwrap_or("").to_string());
        fields.extend(argv.iter().skip(2).cloned());
        crate::ported::params::setsparam("__fish_nfile", &element("__fish_fnfile", &name).unwrap_or_default());
    } else {
        fields.push(argv.get(2).cloned().unwrap_or_default());
    }
    let outer = var("__fish_bt");
    let mut value = fields.join(&sep);
    if !outer.is_empty() {
        value.push('\x1d');
        value.push_str(&outer);
    }
    crate::ported::params::setsparam("__fish_nf", &value);
    status
}

pub fn register_all() {
    crate::native_cmds::register("__fish_frame", |argv| frame(argv));
    crate::native_cmds::register("__fish_error", |argv| run(argv));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_spans_the_word() {
        assert_eq!(describe("msg", "fish", "echo a*b; x", 5, 3, false, false), "fish: msg\necho a*b; x\n     ^~^");
        assert_eq!(describe("", "fish", "nosuchcmd foo", 0, 9, false, false), "fish: \nnosuchcmd foo\n^~~~~~~~^");
        // a tab before the error stays a tab; one character is a lone caret
        assert_eq!(describe("m", "f", "\tx", 1, 1, false, false), "f: m\n\tx\n\t^");
    }

    #[test]
    fn interactive_first_column_has_no_caret() {
        assert_eq!(describe("", "fish", "nosuch", 0, 6, true, true), "");
        assert_eq!(describe("m", "fish", "echo a*", 5, 2, true, false), "fish: m\necho a*\n     ^^");
    }
}
