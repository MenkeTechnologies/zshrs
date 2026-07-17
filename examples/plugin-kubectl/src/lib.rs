//! **kubectl completion** as a native zshrs plugin.
//!
//! This is the native equivalent of a zsh `_kubectl` completion (e.g.
//! nnao45/zsh-kubectl-completion), but instead of a large static, version-
//! pinned `_arguments` tree it delegates to **cobra's built-in completion
//! protocol** — `kubectl __complete <args…> <partial>` — which every modern
//! kubectl exposes. That returns candidates (subcommands, flags, and live
//! resources) for *the installed kubectl version*, one `name\tdescription`
//! per line, terminated by a `:<directive>` line.
//!
//! So the whole completion is: forward the current words to
//! `kubectl __complete` and emit the candidate column. It tracks whatever
//! kubectl is on `PATH`, needs no per-version maintenance, and covers
//! resources dynamically (subject to cluster access, exactly like the zsh
//! completion's `kubectl get` calls).
//!
//! Load after `compinit`; then `kubectl <TAB>`, `kubectl get po<TAB>`, etc.

use std::os::raw::c_int;
use std::process::Command;
use znative::{declare_plugin, Args, Host};

/// compsys generator for `kubectl`. `args.rest()` is `[$CURRENT, word0,
/// word1, …]` where `word0 == "kubectl"` and `$CURRENT` is the 1-based
/// index of the word being completed.
fn kubectl_complete(host: &Host, args: &Args) -> c_int {
    let a = args.rest();
    let Some(current) = a.first().and_then(|s| s.parse::<usize>().ok()) else {
        return 1;
    };
    let words = &a[1..]; // ["kubectl", "get", "po", …]
    if words.is_empty() {
        return 0;
    }

    // cobra wants every arg after the command, with the word being completed
    // as the final arg (possibly empty). `words[0]` is "kubectl".
    let end = current.min(words.len());
    let mut cargs: Vec<String> = vec!["__complete".to_string()];
    cargs.extend(words[1..end].iter().cloned());
    // Completing a fresh word past the last typed one → append the empty
    // partial cobra expects (e.g. `kubectl get <TAB>` → all resource types).
    if current > words.len() {
        cargs.push(String::new());
    }

    let Ok(out) = Command::new("kubectl").args(&cargs).output() else {
        return 0; // kubectl not on PATH — no completions, no error noise.
    };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        // A `:<int>` line is cobra's trailing ShellCompDirective — stop there.
        if line.starts_with(':') {
            break;
        }
        if line.is_empty() {
            continue;
        }
        // "candidate\tdescription" → candidate.
        let cand = line.split('\t').next().unwrap_or(line);
        if !cand.is_empty() {
            host.add_match(cand);
        }
    }
    0
}

declare_plugin! {
    name: "kubectl-completion",
    version: "0.1.0",
    completions: {
        "kubectl" => kubectl_complete,
    },
}
