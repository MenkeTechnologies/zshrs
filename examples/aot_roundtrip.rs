//! Demonstrate the AOT trailer encode/decode pipeline.
//!
//! Usage: `cargo run --example aot_roundtrip` (no args). The example
//! writes a fake "binary" prefix, appends a compressed payload of two
//! embedded zsh scripts via `aot::append_embedded_files`, then reads
//! it back with `aot::try_load_embedded` and prints each script
//! verbatim. Useful for verifying the trailer format end-to-end
//! without having to invoke `zbuild` against a real `zshrs` binary.
//!
//! No C counterpart — see `src/extensions/aot.rs` for the trailer
//! spec; zsh's `zcompile` writes a separate `.zwc` file rather than
//! appending to the executable itself.

use std::io::Write;
use zsh::aot::{append_embedded_files, try_load_embedded, EmbeddedFile};

fn main() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let path = tmp.path().to_path_buf();

    // Pretend `tmp` already contains a compiled zshrs binary.
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .expect("open prefix");
        f.write_all(b"FAKE-ZSHRS-BINARY-PREFIX-DO-NOT-RUN")
            .expect("write prefix");
    }

    let scripts = vec![
        EmbeddedFile {
            name: "01-greet.zsh".to_string(),
            source: "greet() { print -- \"hello, $1\" }\n".to_string(),
        },
        EmbeddedFile {
            name: "02-main.zsh".to_string(),
            source: "greet world\n".to_string(),
        },
    ];

    append_embedded_files(&path, &scripts).expect("append trailer");

    let loaded = try_load_embedded(&path).expect("read trailer back");
    println!("trailer found: {} embedded file(s)", loaded.0.len());
    for (i, f) in loaded.0.iter().enumerate() {
        println!("--- file {} ({} bytes) ---", i, f.source.len());
        println!("name: {}", f.name);
        print!("source:\n{}", f.source);
    }

    assert_eq!(loaded.0.len(), 2);
    assert_eq!(loaded.0[0].name, "01-greet.zsh");
    assert_eq!(loaded.0[1].name, "02-main.zsh");
    println!("roundtrip OK");
}
