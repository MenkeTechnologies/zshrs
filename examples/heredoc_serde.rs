//! Demonstrate the `HereDocInfo` AST node's serde round trip.
//!
//! Usage: `cargo run --example heredoc_serde`. Builds three
//! `HereDocInfo` AST nodes (one unquoted, one with a quoted
//! terminator, one with an empty body), serializes them to JSON,
//! deserializes them back, and prints both forms so a reader can
//! see exactly what the AST layer ships across the wire / disk.
//!
//! Doubles as a regression demo: the `#[serde(default)]` on `quoted`
//! lets older serialized payloads (predating the field) deserialize
//! cleanly — the example exercises that path explicitly.
//!
//! See `src/extensions/heredoc_ast.rs` for the canonical struct
//! definition.

use zsh::heredoc_ast::HereDocInfo;

fn dump(label: &str, info: &HereDocInfo) {
    let json = serde_json::to_string_pretty(info).expect("serialize");
    let back: HereDocInfo = serde_json::from_str(&json).expect("deserialize");
    println!("--- {} ---", label);
    println!("{}", json);
    println!(
        "roundtrip: terminator={:?} quoted={} content_bytes={}",
        back.terminator,
        back.quoted,
        back.content.len()
    );
}

fn main() {
    dump(
        "plain `<<EOF` heredoc (variables expand at runtime)",
        &HereDocInfo {
            content: "user: $USER\nhost: $HOST\n".to_string(),
            terminator: "EOF".to_string(),
            quoted: false,
        },
    );
    dump(
        "quoted `<<'EOF'` heredoc (body is literal)",
        &HereDocInfo {
            content: "user: $USER\nhost: $HOST\n".to_string(),
            terminator: "EOF".to_string(),
            quoted: true,
        },
    );
    dump(
        "empty body",
        &HereDocInfo {
            content: String::new(),
            terminator: "END".to_string(),
            quoted: false,
        },
    );

    // Legacy payload without `quoted` — must default to false.
    let legacy = r#"{"content":"body\n","terminator":"EOF"}"#;
    let info: HereDocInfo = serde_json::from_str(legacy).expect("legacy decode");
    println!("--- legacy payload (missing `quoted`) ---");
    println!("{}", legacy);
    println!(
        "decoded: terminator={:?} quoted={} content={:?}",
        info.terminator, info.quoted, info.content
    );
    assert!(!info.quoted, "default must be false");
}
