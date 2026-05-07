#!/bin/zsh
# Regenerate tests/data/zsh_c_fn_names.txt from the upstream zsh
# C source. Run after pulling new upstream commits.
#
# Usage:
#   ZSH_C_SOURCE=~/forkedRepos/zsh/Src ./tests/data/extract_c_fn_names.sh
# or just:
#   ./tests/data/extract_c_fn_names.sh
#   (defaults to ~/forkedRepos/zsh/Src)
#
# Output format: one entry per line, `<basename>:<fn_name>`. The
# basename is the C file (e.g. `subst.c`) so the drift-detection
# test can verify Rust ports landed in the matching file (rename
# detection).
#
# The list is checked into git so the drift-detection test
# (tests/ported_fn_names_match_c.rs) doesn't depend on a local
# checkout of zsh's source.

set -e
cd "$(dirname "$0")/../.."

ZSH_SRC="${ZSH_C_SOURCE:-$HOME/forkedRepos/zsh/Src}"
if [[ ! -d "$ZSH_SRC" ]]; then
    print -u2 "ERROR: zsh source not found at $ZSH_SRC"
    print -u2 "Set ZSH_C_SOURCE to override."
    exit 1
fi

OUT=tests/data/zsh_c_fn_names.txt

{
    print '# Function names extracted from zsh upstream C source.'
    print '# Format: <basename>:<fn_name>'
    print '# Regenerate via tests/data/extract_c_fn_names.sh.'
    print "# Source: $ZSH_SRC ($(find "$ZSH_SRC" -name "*.c" | wc -l | tr -d ' ') files)"
    print "# Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    print ''

    # For each .c file, extract identifiers preceding `(` and emit
    # `basename:fn`. Strip C keywords. Use awk for the per-file
    # context so we know which file each fn belongs to.
    find "$ZSH_SRC" -name "*.c" -type f | while read -r f; do
        base="${f:t}"
        # Modules sometimes have the same fn in multiple Modules/*.c
        # files (different builtins with overlapping helpers); we
        # keep all occurrences so the test can check "did the port
        # land in any C file containing this fn".
        grep -oE '[a-zA-Z_][a-zA-Z_0-9]*\(' "$f" 2>/dev/null \
            | sed 's/($//' \
            | grep -vE '^(if|while|for|switch|return|sizeof|typedef|do|else)$' \
            | sort -u \
            | sed "s|^|${base}:|"
    done | sort -u
} > "$OUT"

LINES=$(grep -cv '^#' "$OUT" | head -1)
print "Wrote $OUT ($LINES (file,fn) entries)"
