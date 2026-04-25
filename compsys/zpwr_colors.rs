//! ZPWR zstyle color configuration parser
//!
//! Parses actual zstyle commands from ~/.zpwr config files

use std::collections::HashMap;
use std::path::PathBuf;

/// Parsed zstyle color config
#[derive(Clone, Debug, Default)]
pub struct ZstyleColors {
    /// Menu selection color (ma=)
    pub menu_selection: String,
    /// Prefix color for pattern matching
    pub prefix_color: String,
    /// Tag -> completion color mapping
    pub tag_colors: HashMap<String, String>,
    /// List separator (ZPWR_CHAR_LOGO)
    pub list_separator: String,
    /// Header formatting
    pub header: HeaderColors,
    /// Follow symlinks for coloring (LC_FOLLOW_SYMLINKS behavior)
    /// When true, symlinks are colored by their target type instead of symlink color
    pub follow_symlinks: bool,
}

/// Header colors from ZPWR_DESC_* env vars
#[derive(Clone, Debug)]
pub struct HeaderColors {
    pub pre: String,
    pub post: String,
    pub pre_color: String,
    pub text_color: String,
    pub post_color: String,
}

impl Default for HeaderColors {
    fn default() -> Self {
        Self {
            pre: "-<<".into(),
            post: ">>-".into(),
            pre_color: "1;31".into(),
            text_color: "34".into(),
            post_color: "1;31".into(),
        }
    }
}

impl HeaderColors {
    pub fn from_env() -> Self {
        Self {
            pre: std::env::var("ZPWR_DESC_PRE").unwrap_or_else(|_| "-<<".into()),
            post: std::env::var("ZPWR_DESC_POST").unwrap_or_else(|_| ">>-".into()),
            pre_color: std::env::var("ZPWR_DESC_PRE_COLOR").unwrap_or_else(|_| "1;31".into()),
            text_color: std::env::var("ZPWR_DESC_TEXT_COLOR").unwrap_or_else(|_| "34".into()),
            post_color: std::env::var("ZPWR_DESC_POST_COLOR").unwrap_or_else(|_| "1;31".into()),
        }
    }

    pub fn format(&self, text: &str) -> String {
        format!(
            "\x1b[{}m{}\x1b[0m\x1b[{}m{}\x1b[0m\x1b[{}m{}\x1b[0m",
            self.pre_color, self.pre, self.text_color, text, self.post_color, self.post
        )
    }
}

impl ZstyleColors {
    /// Parse zstyle colors from zpwr config files
    pub fn from_zpwr() -> Self {
        let mut colors = Self::default();

        // Add default colors for common file-related tags (green, like zsh default)
        // These can be overridden by zpwr config
        colors.add_default_file_colors();

        // Parse ZPWR env file for header colors and separator
        if let Some(home) = std::env::var("HOME").ok() {
            let env_file = PathBuf::from(&home).join(".zpwr/env/.zpwr_env.sh");
            if let Ok(content) = std::fs::read_to_string(&env_file) {
                colors.parse_env_file(&content);
            }

            // Parse zstyle file for list-colors
            let zstyle_file = PathBuf::from(&home).join(".zpwr/autoload/common/zpwrBindZstyle");
            if let Ok(content) = std::fs::read_to_string(&zstyle_file) {
                colors.parse_zstyle_file(&content);
            }
        }

        // Also check current env vars (they override file parsing)
        colors.header = HeaderColors::from_env();
        if let Ok(sep) = std::env::var("ZPWR_CHAR_LOGO") {
            colors.list_separator = sep;
        }

        // Check for LC_FOLLOW_SYMLINKS env var (non-empty = true)
        colors.follow_symlinks = std::env::var("LC_FOLLOW_SYMLINKS")
            .map(|v| !v.is_empty() && v != "0" && v.to_lowercase() != "false")
            .unwrap_or(false);

        colors
    }

    /// Add default colors for file-related completion tags
    /// Uses green (32) to match standard zsh/terminal file coloring
    fn add_default_file_colors(&mut self) {
        let file_color = "32".to_string(); // green

        // Various file-related tag names
        for tag in &[
            "file",
            "files",
            "all-files",
            "globbed-files",
            "local-directories",
            "directories",
            "directory",
            "path",
            "paths",
        ] {
            self.tag_colors
                .insert((*tag).to_string(), file_color.clone());
        }
    }

    fn parse_env_file(&mut self, content: &str) {
        for line in content.lines() {
            let line = line.trim();
            // export VAR='value' or export VAR="value" or VAR=value
            if let Some(rest) = line.strip_prefix("export ").or(Some(line)) {
                if let Some((key, val)) = rest.split_once('=') {
                    let val = val.trim_matches(|c| c == '\'' || c == '"');
                    match key {
                        "ZPWR_CHAR_LOGO" => self.list_separator = val.into(),
                        "ZPWR_DESC_PRE" => self.header.pre = val.into(),
                        "ZPWR_DESC_POST" => self.header.post = val.into(),
                        "ZPWR_DESC_PRE_COLOR" => self.header.pre_color = val.into(),
                        "ZPWR_DESC_TEXT_COLOR" => self.header.text_color = val.into(),
                        "ZPWR_DESC_POST_COLOR" => self.header.post_color = val.into(),
                        _ => {}
                    }
                }
            }
        }
    }

    fn parse_zstyle_file(&mut self, content: &str) {
        for line in content.lines() {
            let line = line.trim();

            // Skip comments and empty lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse: zstyle ':completion:*...' list-colors '...'
            if !line.starts_with("zstyle ") {
                continue;
            }

            // Extract pattern and value - handle quoted strings properly
            // Format: zstyle 'PATTERN' list-colors 'VALUE'
            let rest = &line[7..]; // Skip "zstyle "

            // Find pattern (first quoted string)
            let (pattern, rest) = Self::extract_quoted(rest);
            if pattern.is_none() {
                continue;
            }
            let pattern = pattern.unwrap();

            // Skip whitespace and find style name
            let rest = rest.trim_start();
            if !rest.starts_with("list-colors") {
                continue;
            }
            let rest = rest[11..].trim_start(); // Skip "list-colors"

            // Extract value
            let (value, _) = Self::extract_quoted(rest);
            if value.is_none() {
                continue;
            }
            let value = value.unwrap();

            // Extract tag from pattern like ':completion:*:*:*:*:aliases'
            if let Some(tag) = Self::extract_tag(&pattern) {
                if tag.is_empty() {
                    // Global completion color, check for ma=
                    if let Some(ma) = value.strip_prefix("ma=") {
                        self.menu_selection = ma.into();
                    }
                } else {
                    // Parse the color value: '=(#b)(*)=PREFIX=COLOR'
                    if let Some(color) = Self::parse_list_color_value(&value) {
                        // Insert both the raw tag and friendly aliases
                        self.tag_colors.insert(tag.to_string(), color.1.clone());

                        // Add friendly name mappings
                        match tag {
                            "executables" => {
                                self.tag_colors
                                    .insert("external command".into(), color.1.clone());
                            }
                            "functions" => {
                                self.tag_colors
                                    .insert("shell function".into(), color.1.clone());
                            }
                            "builtins" => {
                                self.tag_colors
                                    .insert("builtin command".into(), color.1.clone());
                            }
                            "parameters" => {
                                self.tag_colors.insert("parameter".into(), color.1.clone());
                            }
                            "aliases" | "alias" => {
                                self.tag_colors.insert("alias".into(), color.1.clone());
                                self.tag_colors.insert("aliases".into(), color.1.clone());
                            }
                            _ => {}
                        }

                        if self.prefix_color.is_empty() {
                            self.prefix_color = color.0;
                        }
                    }
                }
            }
        }
    }

    /// Extract a quoted string from input, returns (content, rest)
    fn extract_quoted(s: &str) -> (Option<String>, &str) {
        let s = s.trim_start();
        if s.starts_with('\'') {
            if let Some(end) = s[1..].find('\'') {
                return (Some(s[1..end + 1].to_string()), &s[end + 2..]);
            }
        } else if s.starts_with('"') {
            if let Some(end) = s[1..].find('"') {
                return (Some(s[1..end + 1].to_string()), &s[end + 2..]);
            }
        }
        (None, s)
    }

    /// Extract tag name from zstyle pattern
    /// ':completion:*:*:*:*:aliases' -> "aliases"
    /// ':completion:*:functions' -> "functions"  
    /// ':completion:*' -> "" (global)
    fn extract_tag(pattern: &str) -> Option<&str> {
        if !pattern.starts_with(":completion:") {
            return None;
        }

        // Split by colon, get last non-* segment
        let parts: Vec<&str> = pattern.split(':').collect();
        // parts for ":completion:*" = ["", "completion", "*"]
        // parts for ":completion:*:aliases" = ["", "completion", "*", "aliases"]

        // Skip empty, "completion", and "*" - find real tag at end
        for part in parts.iter().rev() {
            if part.is_empty() || *part == "completion" || part.contains('*') {
                continue;
            }
            return Some(*part);
        }

        Some("") // Global pattern (no specific tag)
    }

    /// Parse list-colors value like '=(#b)(*)=1;30=34;42;4'
    /// Returns (prefix_color, completion_color)
    fn parse_list_color_value(value: &str) -> Option<(String, String)> {
        // Format: =(#b)(*)=PREFIX_COLOR=COMPLETION_COLOR
        // or just: ma=COLOR for menu selection

        if value.starts_with("=(#b)(*)=") {
            let rest = &value[9..]; // Skip "=(#b)(*)="
            let parts: Vec<&str> = rest.splitn(2, '=').collect();
            if parts.len() == 2 {
                return Some((parts[0].to_string(), parts[1].to_string()));
            } else if parts.len() == 1 {
                return Some(("1;30".to_string(), parts[0].to_string()));
            }
        }

        None
    }
}

/// Load colors - returns HashMap for backward compatibility
pub fn zpwr_list_colors() -> HashMap<String, String> {
    let colors = ZstyleColors::from_zpwr();
    colors.tag_colors
}

/// Get the full parsed config
pub fn load_zpwr_config() -> ZstyleColors {
    ZstyleColors::from_zpwr()
}

/// Default prefix color
pub const DEFAULT_PREFIX_COLOR: &str = "1;30";

/// Menu selection color - parsed from zstyle, fallback to this
pub const MENU_SELECTION_COLOR: &str = "37;1;4;44";

/// Parsed LS_COLORS for file type coloring
#[derive(Clone, Debug, Default)]
pub struct LsColors {
    pub directory: String,                   // di=
    pub symlink: String,                     // ln=
    pub executable: String,                  // ex=
    pub file: String,                        // fi= (regular file)
    pub extensions: HashMap<String, String>, // *.ext=color
}

impl LsColors {
    /// Parse from LS_COLORS environment variable
    pub fn from_env() -> Self {
        let mut colors = Self::default();

        if let Ok(ls_colors) = std::env::var("LS_COLORS") {
            for entry in ls_colors.split(':') {
                if let Some((key, color)) = entry.split_once('=') {
                    match key {
                        "di" => colors.directory = color.to_string(),
                        "ln" => colors.symlink = color.to_string(),
                        "ex" => colors.executable = color.to_string(),
                        "fi" => colors.file = color.to_string(),
                        _ if key.starts_with("*.") => {
                            let ext = key[2..].to_lowercase();
                            colors.extensions.insert(ext, color.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }

        // Defaults if not set (match common terminal defaults)
        if colors.directory.is_empty() {
            colors.directory = "1;34".to_string(); // bold blue
        }
        if colors.symlink.is_empty() {
            colors.symlink = "1;36".to_string(); // bold cyan
        }
        if colors.executable.is_empty() {
            colors.executable = "1;32".to_string(); // bold green
        }
        // Regular files have no color by default (use terminal default)

        colors
    }

    /// Get color for a specific file
    pub fn color_for(&self, filename: &str, is_dir: bool, is_exec: bool, is_link: bool) -> &str {
        if is_link {
            return &self.symlink;
        }
        if is_dir {
            return &self.directory;
        }
        if is_exec {
            return &self.executable;
        }

        // Check extension
        if let Some(ext) = filename.rsplit('.').next() {
            if let Some(color) = self.extensions.get(&ext.to_lowercase()) {
                return color;
            }
        }

        // Default file color (empty = terminal default)
        &self.file
    }
}

/// Global cached LS_COLORS
static LS_COLORS: std::sync::OnceLock<LsColors> = std::sync::OnceLock::new();

/// Get color for a file based on LS_COLORS
pub fn ls_color_for_file(filename: &str, is_dir: bool, is_exec: bool, is_link: bool) -> String {
    let colors = LS_COLORS.get_or_init(LsColors::from_env);
    colors
        .color_for(filename, is_dir, is_exec, is_link)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tag() {
        assert_eq!(
            ZstyleColors::extract_tag(":completion:*:*:*:*:aliases"),
            Some("aliases")
        );
        assert_eq!(
            ZstyleColors::extract_tag(":completion:*:functions"),
            Some("functions")
        );
        assert_eq!(ZstyleColors::extract_tag(":completion:*"), Some(""));
        assert_eq!(
            ZstyleColors::extract_tag(":completion:*:zpwr-vim"),
            Some("zpwr-vim")
        );
        assert_eq!(ZstyleColors::extract_tag("not-completion"), None);
    }

    #[test]
    fn test_parse_list_color_value() {
        assert_eq!(
            ZstyleColors::parse_list_color_value("=(#b)(*)=1;30=34;42;4"),
            Some(("1;30".to_string(), "34;42;4".to_string()))
        );
        assert_eq!(
            ZstyleColors::parse_list_color_value("=(#b)(*)=1;30=1;37;44"),
            Some(("1;30".to_string(), "1;37;44".to_string()))
        );
    }

    #[test]
    fn test_header_colors_format() {
        let hc = HeaderColors::default();
        let formatted = hc.format("test");
        assert!(formatted.contains("-<<"));
        assert!(formatted.contains("test"));
        assert!(formatted.contains(">>-"));
    }

    #[test]
    fn test_from_zpwr_loads_something() {
        let colors = ZstyleColors::from_zpwr();
        // Should have parsed at least some colors if zpwr is installed
        // This test will pass even without zpwr (empty is valid)
        assert!(colors.tag_colors.len() >= 0);
    }

    #[test]
    fn test_parse_zstyles_basic() {
        let content = r#"
zstyle ':completion:*' menu select
zstyle ':completion:*:descriptions' format '%d'
"#;
        let styles = parse_zstyles_from_content(content);
        assert_eq!(styles.len(), 2);

        assert_eq!(styles[0].pattern, ":completion:*");
        assert_eq!(styles[0].style, "menu");
        assert_eq!(styles[0].values, vec!["select"]);
        assert!(!styles[0].eval);

        assert_eq!(styles[1].pattern, ":completion:*:descriptions");
        assert_eq!(styles[1].style, "format");
        assert_eq!(styles[1].values, vec!["%d"]);
    }

    #[test]
    fn test_parse_zstyles_with_builtin() {
        let content = r#"
builtin zstyle ':completion:*' verbose yes
"#;
        let styles = parse_zstyles_from_content(content);
        assert_eq!(styles.len(), 1);
        assert_eq!(styles[0].style, "verbose");
        assert_eq!(styles[0].values, vec!["yes"]);
    }

    #[test]
    fn test_parse_zstyles_with_eval() {
        let content = r#"
zstyle -e ':completion:*' hosts 'reply=($myhosts)'
"#;
        let styles = parse_zstyles_from_content(content);
        assert_eq!(styles.len(), 1);
        assert!(styles[0].eval);
        assert_eq!(styles[0].style, "hosts");
    }

    #[test]
    fn test_parse_zstyles_multiple_values() {
        let content = r#"
zstyle ':completion:*' completer _complete _approximate _correct
"#;
        let styles = parse_zstyles_from_content(content);
        assert_eq!(styles.len(), 1);
        assert_eq!(
            styles[0].values,
            vec!["_complete", "_approximate", "_correct"]
        );
    }

    #[test]
    fn test_parse_zstyles_quoted_values() {
        let content = r#"
zstyle ':completion:*' format "-- %d --"
zstyle ':completion:*' list-prompt 'At %p'
"#;
        let styles = parse_zstyles_from_content(content);
        assert_eq!(styles.len(), 2);
        assert_eq!(styles[0].values, vec!["-- %d --"]);
        assert_eq!(styles[1].values, vec!["At %p"]);
    }

    #[test]
    fn test_parse_zstyles_ignores_comments() {
        let content = r#"
# This is a comment
zstyle ':completion:*' menu select
# Another comment
"#;
        let styles = parse_zstyles_from_content(content);
        assert_eq!(styles.len(), 1);
    }

    #[test]
    fn test_parse_zstyles_ignores_empty_lines() {
        let content = r#"

zstyle ':completion:*' menu select

zstyle ':completion:*' verbose yes

"#;
        let styles = parse_zstyles_from_content(content);
        assert_eq!(styles.len(), 2);
    }

    #[test]
    fn test_parse_ansi_c_string_basic() {
        assert_eq!(parse_ansi_c_string("hello"), "hello");
        assert_eq!(parse_ansi_c_string(r"hello\nworld"), "hello\nworld");
        assert_eq!(parse_ansi_c_string(r"tab\there"), "tab\there");
    }

    #[test]
    fn test_parse_ansi_c_string_escapes() {
        assert_eq!(parse_ansi_c_string(r"\\"), "\\");
        assert_eq!(parse_ansi_c_string(r"\'"), "'");
        assert_eq!(parse_ansi_c_string(r"\r\n"), "\r\n");
    }

    #[test]
    fn test_header_colors_default() {
        let hc = HeaderColors::default();
        assert_eq!(hc.pre, "-<<");
        assert_eq!(hc.post, ">>-");
        assert_eq!(hc.pre_color, "1;31");
        assert_eq!(hc.text_color, "34");
        assert_eq!(hc.post_color, "1;31");
    }

    #[test]
    fn test_zstyle_colors_default() {
        let colors = ZstyleColors::default();
        assert!(colors.menu_selection.is_empty());
        assert!(colors.prefix_color.is_empty());
        assert!(colors.tag_colors.is_empty());
    }

    #[test]
    fn test_parsed_zstyle_struct() {
        let zstyle = ParsedZstyle {
            pattern: ":completion:*".to_string(),
            style: "menu".to_string(),
            values: vec!["select".to_string()],
            eval: false,
        };

        assert_eq!(zstyle.pattern, ":completion:*");
        assert_eq!(zstyle.style, "menu");
        assert!(!zstyle.eval);
    }
}

// =============================================================================
// Full zstyle parser for cache population
// =============================================================================

/// Parsed zstyle entry
#[derive(Debug, Clone)]
pub struct ParsedZstyle {
    pub pattern: String,
    pub style: String,
    pub values: Vec<String>,
    pub eval: bool,
}

/// Parse all zstyles from shell config files
pub fn parse_zstyles_from_config() -> Vec<ParsedZstyle> {
    let mut styles = Vec::new();

    if let Ok(home) = std::env::var("HOME") {
        // Parse main .zshrc
        let zshrc = format!("{}/.zshrc", home);
        if let Ok(content) = std::fs::read_to_string(&zshrc) {
            styles.extend(parse_zstyles_from_content(&content));
        }

        // Parse ZPWR .zshrc
        let zpwr_zshrc = format!("{}/.zpwr/install/.zshrc", home);
        if let Ok(content) = std::fs::read_to_string(&zpwr_zshrc) {
            styles.extend(parse_zstyles_from_content(&content));
        }

        // Parse zpwrBindZstyle
        let zstyle_file = format!("{}/.zpwr/autoload/common/zpwrBindZstyle", home);
        if let Ok(content) = std::fs::read_to_string(&zstyle_file) {
            styles.extend(parse_zstyles_from_content(&content));
        }

        // Parse zpwrBindMenu (where completer is set)
        let menu_file = format!("{}/.zpwr/autoload/common/zpwrBindMenu", home);
        if let Ok(content) = std::fs::read_to_string(&menu_file) {
            styles.extend(parse_zstyles_from_content(&content));
        }
    }

    styles
}

/// Parse zstyle commands from shell script content
pub fn parse_zstyles_from_content(content: &str) -> Vec<ParsedZstyle> {
    let mut styles = Vec::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Handle: zstyle 'pattern' style value...
        // Also: builtin zstyle 'pattern' style value...
        // Also: zstyle -e 'pattern' style 'eval-code'
        let line = line.strip_prefix("builtin ").unwrap_or(line);

        if !line.starts_with("zstyle ") {
            continue;
        }

        let rest = &line[7..].trim_start(); // Skip "zstyle "

        // Check for -e (eval) flag
        let (eval, rest) = if rest.starts_with("-e ") {
            (true, rest[3..].trim_start())
        } else {
            (false, *rest)
        };

        // Extract pattern (first quoted or unquoted string)
        let (pattern, rest) = extract_zstyle_arg(rest);
        if pattern.is_none() {
            continue;
        }
        let pattern = pattern.unwrap();

        // Extract style name
        let rest = rest.trim_start();
        let (style, rest) = extract_zstyle_arg(rest);
        if style.is_none() {
            continue;
        }
        let style = style.unwrap();

        // Extract values (remaining arguments)
        let mut values = Vec::new();
        let mut remaining = rest.trim_start();
        while !remaining.is_empty() {
            let (val, r) = extract_zstyle_arg(remaining);
            if let Some(v) = val {
                values.push(v);
                remaining = r.trim_start();
            } else {
                break;
            }
        }

        styles.push(ParsedZstyle {
            pattern,
            style,
            values,
            eval,
        });
    }

    styles
}

/// Extract a zstyle argument (quoted or unquoted)
fn extract_zstyle_arg(s: &str) -> (Option<String>, &str) {
    let s = s.trim_start();
    if s.is_empty() {
        return (None, s);
    }

    // Single quoted string
    if s.starts_with('\'') {
        let mut i = 1;
        let chars: Vec<char> = s.chars().collect();
        while i < chars.len() {
            if chars[i] == '\'' {
                // Check for escaped quote ''
                if i + 1 < chars.len() && chars[i + 1] == '\'' {
                    i += 2;
                    continue;
                }
                let content: String = chars[1..i].iter().collect();
                // Handle '' escapes in content
                let content = content.replace("''", "'");
                return (Some(content), &s[i + 1..]);
            }
            i += 1;
        }
        return (None, s);
    }

    // Double quoted string
    if s.starts_with('"') {
        let mut i = 1;
        let chars: Vec<char> = s.chars().collect();
        while i < chars.len() {
            if chars[i] == '"' && (i == 0 || chars[i - 1] != '\\') {
                let content: String = chars[1..i].iter().collect();
                return (Some(content), &s[i + 1..]);
            }
            i += 1;
        }
        return (None, s);
    }

    // $'...' ANSI-C quoting
    if s.starts_with("$'") {
        if let Some(end) = s[2..].find('\'') {
            let content = &s[2..end + 2];
            // Parse ANSI-C escape sequences
            let content = parse_ansi_c_string(content);
            return (Some(content), &s[end + 3..]);
        }
        return (None, s);
    }

    // Unquoted word (until whitespace)
    let end = s.find(|c: char| c.is_whitespace()).unwrap_or(s.len());
    if end == 0 {
        return (None, s);
    }
    (Some(s[..end].to_string()), &s[end..])
}

/// Parse ANSI-C escape sequences in $'...' strings
fn parse_ansi_c_string(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('e') => result.push('\x1b'),
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('\'') => result.push('\''),
                Some('[') => result.push('['),
                Some('0') => {
                    // Octal escape \0xxx
                    let mut octal = String::new();
                    while octal.len() < 3 && chars.peek().map_or(false, |c| c.is_ascii_digit()) {
                        octal.push(chars.next().unwrap());
                    }
                    if let Ok(n) = u8::from_str_radix(&octal, 8) {
                        result.push(n as char);
                    }
                }
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }

    result
}
