//! Every zshrs builtin must ship a completion.
//!
//! `ZSHRS_BUILTIN_NAMES` (daemon/builtins.rs) is the registry the shell
//! itself dispatches on, and its doc comment already names completion as
//! a consumer. `completions/` is bundled into `~/.zshrs/functions` by
//! build.rs, so a builtin without a file there simply has no completion
//! at the prompt -- which is how `zjob <TAB>` did nothing while `zd` had
//! a hand-written one.
//!
//! Source-level on purpose: it fails the moment a builtin is added
//! without its completion, rather than at someone's prompt.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn registry_names() -> Vec<String> {
    let src = std::fs::read_to_string(repo_root().join("daemon/builtins.rs"))
        .expect("read daemon/builtins.rs");
    let start = src
        .find("ZSHRS_BUILTIN_NAMES")
        .expect("ZSHRS_BUILTIN_NAMES not found — did the registry move?");
    let open = src[start..].find('[').expect("registry opening bracket") + start;
    let close = src[open..].find("];").expect("registry closing bracket") + open;
    let body = &src[open..close];
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find('"') {
        rest = &rest[i + 1..];
        match rest.find('"') {
            Some(j) => {
                out.push(rest[..j].to_string());
                rest = &rest[j + 1..];
            }
            None => break,
        }
    }
    out
}

#[test]
fn every_builtin_has_a_bundled_completion() {
    let names = registry_names();
    assert!(
        names.len() >= 20,
        "registry parse looks wrong, got {names:?}"
    );
    let dir = repo_root().join("completions");
    let missing: Vec<&String> = names
        .iter()
        .filter(|n| !dir.join(format!("_{n}")).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "these builtins have no completion in completions/: {missing:?}"
    );
}

/// The completions have to be syntactically valid zsh, otherwise compinit
/// silently drops them and the builtin completes as if nothing shipped.
#[test]
fn bundled_completions_declare_their_command() {
    for n in registry_names() {
        let p = repo_root().join("completions").join(format!("_{n}"));
        let body = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));
        let first = body.lines().next().unwrap_or("");
        assert_eq!(
            first,
            format!("#compdef {n}"),
            "{p:?} must open with its #compdef tag so compinit binds it"
        );
    }
}

/// `zwhere` answers without a daemon (it reads the recorder shard), so its
/// completion must not wait for `daemon.sock`, and must pass its query as a
/// quoted array: unquoted, `--prefix ""` (nothing typed yet) loses its empty
/// argument and `zwhere function <TAB>` queries the wrong thing.
#[test]
fn zwhere_completion_does_not_need_the_daemon_and_keeps_empty_args() {
    let src = std::fs::read_to_string(repo_root().join("completions/_zwhere"))
        .expect("read completions/_zwhere");
    assert!(!src.contains("daemon.sock"), "_zwhere must not gate on the daemon socket");
    assert!(
        !src.contains("zwhere $query"),
        "_zwhere must pass the query as \"${{query[@]}}\" so an empty --prefix survives"
    );
    assert!(src.contains("zwhere \"${query[@]}\""));
}
