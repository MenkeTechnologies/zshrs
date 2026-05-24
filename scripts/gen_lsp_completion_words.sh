#!/usr/bin/env bash
# Regenerate `completions/lsp_completion_words.txt` — the canonical
# sorted-ASCII snapshot of every name LSP tab-complete should know
# about (mirror of stryke's `lsp_completion_words.txt`).
#
# Three sources merged via `zshrs --names`:
#   1. Canonical builtins (`ported::builtin::BUILTINS`)
#   2. Canonical reserved words (`ported::hashtable::RESWDS`)
#   3. Options / specials / compsys fns / extension builtins / operators
#
# The file is consumed by the `_zshrs` completer for `--docs <TAB>`,
# so the shell doesn't have to spawn `zshrs --names` on every tab
# press (cold-start of the binary loads ~30 MB of bytecode caches).
#
# Re-run after adding builtins / options / extension fns.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --quiet --bin zshrs
out=completions/lsp_completion_words.txt
target/debug/zshrs --names > "$out"
echo "wrote $out — $(wc -l < "$out" | tr -d ' ') entries"
