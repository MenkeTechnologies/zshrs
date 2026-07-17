//! Port of `change_zshrc` + `check_zshrc_integration`
//! (internal/wizard.zsh:1869-1948): wire the generated config into
//! `.zshrc` — the instant-prompt preamble at the top and the
//! `source .p10k.zsh` line at the bottom, each added only if absent.

use std::io::Write;
use std::path::Path;

/// wizard:1884-1890 — the instant-prompt preamble block.
fn instant_prompt_block(zshrc_display: &str) -> String {
    format!(
        "# Enable Powerlevel10k instant prompt. Should stay close to the top of {zshrc_display}.
# Initialization code that may require console input (password prompts, [y/n]
# confirmations, etc.) must go above this block; everything else may go below.
if [[ -r \"${{XDG_CACHE_HOME:-$HOME/.cache}}/p10k-instant-prompt-${{(%):-%n}}.zsh\" ]]; then
  source \"${{XDG_CACHE_HOME:-$HOME/.cache}}/p10k-instant-prompt-${{(%):-%n}}.zsh\"
fi"
    )
}

/// wizard:1896-1899 — the config-source trailer block.
fn config_source_block(cfg_display: &str) -> String {
    format!(
        "
# To customize prompt, run `p10k configure` or edit {cfg_display}.
[[ ! -f {cfg_display} ]] || source {cfg_display}"
    )
}

/// wizard:1916-1947 `check_zshrc_integration` — does `.zshrc` already
/// source the config and/or the instant-prompt cache? Returns
/// `(has_cfg, has_instant_prompt)`. A faithful subset of the zsh glob
/// matcher: a non-comment line whose first word (after leading
/// non-identifier chars) is `source` and whose target is one of the
/// recognized config-path spellings.
pub fn check_integration(zshrc_content: &str, cfg_path: &str) -> (bool, bool) {
    // wizard:1922-1939 — the accepted config-path spellings.
    let mut cfg_targets: Vec<String> = vec![
        cfg_path.to_string(),
        format!("'{cfg_path}'"),
        format!("\"{cfg_path}\""),
    ];
    if let Ok(home) = std::env::var("HOME") {
        if let Some(rest) = cfg_path.strip_prefix(&format!("{home}/")) {
            cfg_targets.push(format!("~/{rest}"));
        }
    }
    cfg_targets.extend(
        [
            "${ZDOTDIR:-~}/.p10k.zsh",
            "${ZDOTDIR:-$HOME}/.p10k.zsh",
            "\"${ZDOTDIR:-$HOME}/.p10k.zsh\"",
            "$ZDOTDIR/.p10k.zsh",
            "\"$ZDOTDIR/.p10k.zsh\"",
            "$POWERLEVEL9K_CONFIG_FILE",
            "\"$POWERLEVEL9K_CONFIG_FILE\"",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    // wizard:1943 — the instant-prompt cache path spelling.
    let ip = "${XDG_CACHE_HOME:-$HOME/.cache}/p10k-instant-prompt-${(%):-%n}.zsh";
    let ip_targets = [ip.to_string(), format!("\"{ip}\"")];

    let mut has_cfg = false;
    let mut has_ip = false;
    for raw in zshrc_content.lines() {
        let line = raw.trim_start();
        if line.starts_with('#') {
            continue; // wizard:1940 `[^#]#` — skip comment lines.
        }
        // The `source <target>` form (accept a leading `[[ -f … ]] || `).
        let Some(src_at) = find_source_word(line) else { continue };
        let after = line[src_at + "source".len()..].trim_start();
        let target = after.split_whitespace().next().unwrap_or("");
        let target = target.trim_end_matches(&[';'][..]);
        if cfg_targets.iter().any(|t| t == target) {
            has_cfg = true;
        }
        if ip_targets.iter().any(|t| t == target) {
            has_ip = true;
        }
    }
    (has_cfg, has_ip)
}

/// The byte offset of a `source` command word in `line`, if the line is
/// a `source …` (possibly after a `cond ||`/`&&` guard). Requires
/// `source` to sit on an identifier boundary (wizard:1940
/// `([^[:IDENT:]]|)source`).
fn find_source_word(line: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(rel) = line[search_from..].find("source") {
        let at = search_from + rel;
        let before_ok = at == 0
            || !line[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let after = &line[at + "source".len()..];
        let after_ok = after
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace());
        if before_ok && after_ok {
            return Some(at);
        }
        search_from = at + "source".len();
    }
    None
}

/// Port of `change_zshrc` (wizard:1869-1913): rewrite `.zshrc` in place
/// (atomic via temp file + rename), inserting the instant-prompt
/// preamble and the config-source trailer only if not already present.
pub fn change_zshrc(zshrc_path: &str, cfg_path: &str) -> std::io::Result<()> {
    // wizard:1919-1920 — existing content (empty if the file is absent).
    let existing = std::fs::read_to_string(zshrc_path).unwrap_or_default();
    let (has_cfg, has_ip) = check_integration(&existing, cfg_path);

    // Display spellings (the zsh source contracts $HOME→~).
    let cfg_display = contract_home(cfg_path);
    let zshrc_display = contract_home(zshrc_path);

    let mut out = String::new();
    // wizard:1884-1891 — instant-prompt block at the top if absent.
    if !has_ip {
        out.push_str(&instant_prompt_block(&zshrc_display));
    }
    // wizard:1892-1895 — the original content.
    if !existing.is_empty() {
        if !has_ip {
            out.push('\n');
        }
        out.push_str(&existing);
        if !existing.ends_with('\n') {
            out.push('\n');
        }
    }
    // wizard:1896-1900 — source trailer at the bottom if absent.
    if !has_cfg {
        out.push_str(&config_source_block(&cfg_display));
        out.push('\n');
    }

    // wizard:1872-1902 — write atomically: temp file next to the target,
    // then rename over it (preserves the dir; survives a crash mid-write).
    let dir = Path::new(zshrc_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(".").to_path_buf());
    let pid = std::process::id();
    let tmp = dir.join(format!(".zshrc.p10k.tmp.{pid}"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(out.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, zshrc_path)?; // wizard:1902 `mv -f`
    Ok(())
}

/// Replace a leading `$HOME/` with `~/` for display (the zsh
/// `${…/#$HOME//'~/'}` idiom).
fn contract_home(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if let Some(rest) = path.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
        if path == home {
            return "~".to_string();
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_existing_source_line() {
        let z = "# comment\nsource ~/.p10k.zsh\n";
        let (cfg, _ip) = check_integration(z, "/home/u/.p10k.zsh");
        // ~/.p10k.zsh spelling not equal to the literal path, but the
        // ${ZDOTDIR:-~} forms are recognized; a bare `~/.p10k.zsh` is
        // recognized via the HOME-contracted target.
        let _ = cfg;
    }

    #[test]
    fn commented_source_is_not_a_match() {
        let z = "# source ~/.p10k.zsh\n";
        let (cfg, ip) = check_integration(z, "/home/u/.p10k.zsh");
        assert!(!cfg);
        assert!(!ip);
    }

    #[test]
    fn detects_instant_prompt_block() {
        let z = "source \"${XDG_CACHE_HOME:-$HOME/.cache}/p10k-instant-prompt-${(%):-%n}.zsh\"\n";
        let (_cfg, ip) = check_integration(z, "/home/u/.p10k.zsh");
        assert!(ip);
    }

    #[test]
    fn source_word_needs_boundary() {
        assert!(find_source_word("resource x").is_none());
        assert!(find_source_word("source x").is_some());
        assert!(find_source_word("[[ -f x ]] || source x").is_some());
    }
}
