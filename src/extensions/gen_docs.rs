//! Module-doc generator — produces Markdown documentation from a
//! parsed zshrs source file by pairing `##` doc comments with the
//! top-level function declaration immediately below them.
//!
//! Driven by `zshrs --gen-docs [PATH] [--out DIR]`, which walks a
//! directory tree and calls [`generate_markdown`] once per file.
//!
//! Mirrors strykelang's `strykelang::docs::generate_markdown` shape
//! (`# Module: …` header → per-section blocks). Sections covered:
//!
//!   * **Header doc** — leading `##` block at top of file.
//!   * **Functions** — `function NAME { … }` and `NAME() { … }`.
//!     Includes `##` doc-comment runs immediately above the decl.
//!   * **Aliases** — `alias NAME=…`.
//!   * **Exports** — `export NAME=…` / `readonly NAME=…`.
//!
//! Anything not matched is silently skipped; the goal is human-
//! readable per-file reference docs, not a full AST dump.

use std::path::Path;

/// Emit Markdown for every top-level declaration in `program` that
/// looks like a public API surface (function / alias / export). Doc
/// comments are extracted from the SOURCE — consecutive `##` lines
/// immediately above the declaration line — so the parser doesn't
/// need to track them.
pub fn generate_markdown(filename: &str, source: &str) -> String {
    let source_lines: Vec<&str> = source.lines().collect();
    let module_title = derive_module_title(filename);

    let mut out = String::new();
    out.push_str("# Module: ");
    out.push_str(&module_title);
    out.push_str("\n\n");

    // Module-level header doc: any `##` block at the very top of the
    // file, before the first non-blank non-comment line.
    if let Some(header) = leading_module_doc(&source_lines) {
        out.push_str(&header);
        out.push_str("\n\n");
    }

    // Walk the source for top-level decls. We don't use the full
    // AST here because zshrs's parser is built for execution, not
    // module-doc extraction — line-based regex matching is more
    // robust for the patterns we care about.
    let mut funcs: Vec<FuncDecl> = Vec::new();
    let mut aliases: Vec<AliasDecl> = Vec::new();
    let mut exports: Vec<ExportDecl> = Vec::new();

    for (i, line) in source_lines.iter().enumerate() {
        let t = line.trim_start();
        // `function NAME { … }` or `function NAME() { … }`
        if let Some(rest) = t.strip_prefix("function ").or_else(|| t.strip_prefix("function\t")) {
            if let Some(name) = first_fn_name(rest) {
                funcs.push(FuncDecl {
                    name,
                    decl_line: i + 1,
                });
                continue;
            }
        }
        // `NAME() { … }` — implicit-function form. Only top-level
        // (column 0 / leading-whitespace-only — already trimmed).
        if let Some(idx) = t.find("()") {
            let head = &t[..idx];
            if is_simple_identifier(head)
                && !head.is_empty()
                && line
                    .chars()
                    .take_while(|c| c.is_whitespace())
                    .next()
                    .map(|c| c == ' ' || c == '\t')
                    .unwrap_or(true)
            {
                funcs.push(FuncDecl {
                    name: head.to_string(),
                    decl_line: i + 1,
                });
                continue;
            }
        }
        // `alias NAME=…` / `alias -g NAME=…` / `alias -s NAME=…`
        if let Some(rest) = t.strip_prefix("alias ") {
            let rest = rest.trim_start_matches("-g ").trim_start_matches("-s ");
            if let Some(eq) = rest.find('=') {
                let name = rest[..eq].trim().to_string();
                if !name.is_empty() {
                    aliases.push(AliasDecl {
                        name,
                        rhs: rest[eq + 1..].trim().to_string(),
                        decl_line: i + 1,
                    });
                    continue;
                }
            }
        }
        // `export NAME=…` / `readonly NAME=…`
        for verb in &["export ", "readonly ", "typeset -gx "] {
            if let Some(rest) = t.strip_prefix(verb) {
                if let Some(eq) = rest.find('=') {
                    let name = rest[..eq].trim().to_string();
                    if !name.is_empty() {
                        exports.push(ExportDecl {
                            name,
                            verb: (*verb).trim().to_string(),
                            decl_line: i + 1,
                        });
                    }
                }
            }
        }
    }

    if !funcs.is_empty() {
        out.push_str("## Functions\n\n");
        for f in &funcs {
            out.push_str(&format!("### `{}`\n\n", f.name));
            let doc = extract_doc_above(&source_lines, f.decl_line);
            if !doc.is_empty() {
                out.push_str(&doc);
                out.push_str("\n\n");
            } else {
                out.push_str("_Defined at line ");
                out.push_str(&f.decl_line.to_string());
                out.push_str("._\n\n");
            }
        }
    }

    if !aliases.is_empty() {
        out.push_str("## Aliases\n\n");
        for a in &aliases {
            out.push_str(&format!("- `{}` = `{}`\n", a.name, a.rhs));
        }
        out.push('\n');
    }

    if !exports.is_empty() {
        out.push_str("## Exports / Constants\n\n");
        for e in &exports {
            out.push_str(&format!("- `{}` ({}) — line {}\n", e.name, e.verb, e.decl_line));
        }
        out.push('\n');
    }

    if funcs.is_empty() && aliases.is_empty() && exports.is_empty() {
        out.push_str("_No public-API decls found in this file._\n");
    }

    out
}

struct FuncDecl {
    name: String,
    decl_line: usize,
}

struct AliasDecl {
    name: String,
    rhs: String,
    decl_line: usize,
}

struct ExportDecl {
    name: String,
    verb: String,
    decl_line: usize,
}

fn derive_module_title(filename: &str) -> String {
    Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| filename.to_string())
}

fn leading_module_doc(lines: &[&str]) -> Option<String> {
    let mut buf = Vec::new();
    for l in lines {
        let t = l.trim_start();
        if t.is_empty() {
            if !buf.is_empty() {
                return Some(buf.join("\n"));
            }
            continue;
        }
        if let Some(body) = t.strip_prefix("##") {
            // Strip at most one leading space — `## text` → `text`.
            let body = body.strip_prefix(' ').unwrap_or(body);
            buf.push(body.to_string());
            continue;
        }
        // Skip plain `#` comments and shebangs (don't count as doc,
        // don't terminate the doc-block search either — they're just
        // header noise).
        if t.starts_with("#!") || t.starts_with('#') {
            continue;
        }
        break;
    }
    if buf.is_empty() {
        None
    } else {
        Some(buf.join("\n"))
    }
}

fn extract_doc_above(lines: &[&str], decl_line_1based: usize) -> String {
    let mut buf: Vec<String> = Vec::new();
    let mut i = decl_line_1based.saturating_sub(1);
    // Walk upward collecting contiguous `##` lines. Stop at the first
    // non-`##` line.
    while i > 0 {
        i -= 1;
        let t = lines[i].trim_start();
        if let Some(body) = t.strip_prefix("##") {
            let body = body.strip_prefix(' ').unwrap_or(body);
            buf.push(body.to_string());
            continue;
        }
        break;
    }
    buf.reverse();
    buf.join("\n")
}

fn first_fn_name(rest: &str) -> Option<String> {
    let s = rest.trim_start();
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':'))
        .unwrap_or(s.len());
    if end == 0 {
        None
    } else {
        Some(s[..end].to_string())
    }
}

fn is_simple_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':' || c == '+' || c == '@')
}

/// Walk `root` and collect every shell-source file path. Same set the
/// IntelliJ plugin recognizes: `.zsh`, `.sh`, `.bash`, `.ksh`, and the
/// canonical dotfile names.
pub fn collect_doc_sources(root: &Path, out: &mut Vec<std::path::PathBuf>) {
    if root.is_file() {
        if is_shell_source(root) {
            out.push(root.to_path_buf());
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        // Skip noisy dirs.
        if matches!(name, ".git" | "node_modules" | "target" | "build" | "dist") {
            continue;
        }
        if p.is_dir() {
            collect_doc_sources(&p, out);
        } else if is_shell_source(&p) {
            out.push(p);
        }
    }
}

fn is_shell_source(p: &Path) -> bool {
    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if matches!(
        name,
        ".zshrc" | ".zshenv" | ".zlogin" | ".zlogout" | ".zprofile" | ".zpreztorc"
            | ".bashrc" | ".bash_profile" | ".profile"
    ) {
        return true;
    }
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
    matches!(ext, "zsh" | "sh" | "bash" | "ksh")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_function_decls_and_doc_comments() {
        let src = "## Top-level header doc for the module.\n\
                   ## Spans multiple lines.\n\
                   \n\
                   ## Greet someone.\n\
                   ## Prints `hello NAME` to stdout.\n\
                   greet() {\n\
                   \techo hello $1\n\
                   }\n\
                   \n\
                   alias ll='ls -al'\n\
                   export FOO=bar\n";
        let md = generate_markdown("greet.zsh", src);
        assert!(md.contains("# Module: greet"), "missing module title: {}", md);
        assert!(md.contains("Top-level header doc"), "missing header doc: {}", md);
        assert!(md.contains("### `greet`"), "missing function entry: {}", md);
        assert!(md.contains("Greet someone"), "missing per-fn doc: {}", md);
        assert!(md.contains("- `ll` = `'ls -al'`"), "missing alias: {}", md);
        assert!(md.contains("- `FOO` (export)"), "missing export: {}", md);
    }

    #[test]
    fn empty_file_yields_no_decls_marker() {
        let md = generate_markdown("empty.zsh", "");
        assert!(md.contains("No public-API decls"), "{}", md);
    }

    #[test]
    fn function_keyword_form_is_recognized() {
        let src = "function foo {\n  echo hi\n}\n";
        let md = generate_markdown("foo.zsh", src);
        assert!(md.contains("### `foo`"), "{}", md);
    }
}
