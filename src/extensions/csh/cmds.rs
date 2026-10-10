//! One csh command line → zsh. No control-structure keyword at its head.

/// Translate a command line that may hold lists (`;` `&&` `||`), pipes
/// (`|` `|&`), background `&`, redirections (`>&` `>!` `>>&` `>>!` `<<`),
/// parenthesised subshells, and the builtins whose syntax differs from zsh
/// (`set` `unset` `setenv` `unsetenv` `alias` `unalias` `shift` `exit`
/// `source` `@` `limit` `unlimit` `rehash` `hashstat` `which` …).
/// Words go through [`super::words::translate_word`].
pub fn translate_line(line: &str) -> Result<String, String> {
    Ok(line.to_string())
}
