#!/bin/bash
# zshrs corpus test runner
# Tests zshrs against real zsh for plugin corpus (when zsh available)
# Falls back to syntax checking when zsh is not available

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ZSHRS="${ZSHRS:-$SCRIPT_DIR/../../target/debug/zshrs}"
CORPUS_DIR="$SCRIPT_DIR"
TIMEOUT=5
FAILURES_LOG="$SCRIPT_DIR/corpus_failures.log"
SUMMARY_ONLY="${SUMMARY_ONLY:-false}"

: > "$FAILURES_LOG"

pass=0
fail=0
skip=0
total=0

# Check if real zsh is available
HAS_ZSH=false
if command -v zsh &>/dev/null; then
    HAS_ZSH=true
fi

for zsh_file in "$CORPUS_DIR"/*.zsh; do
    [[ -f "$zsh_file" ]] || continue
    name=$(basename "$zsh_file")
    total=$((total + 1))
    
    if $HAS_ZSH; then
        # Compare against real zsh
        if ! expected=$(timeout "$TIMEOUT" zsh "$zsh_file" 2>&1); then
            # Script errors/hangs in real zsh - skip
            [[ "$SUMMARY_ONLY" != "true" ]] && echo "SKIP $name (zsh failed/timeout)"
            skip=$((skip + 1))
            continue
        fi
        
        if ! actual=$(timeout "$TIMEOUT" "$ZSHRS" "$zsh_file" 2>&1); then
            [[ "$SUMMARY_ONLY" != "true" ]] && echo "FAIL $name (zshrs failed/timeout)"
            {
                echo "=== $name ==="
                echo "zshrs exit: timeout or error"
                echo "expected:"
                echo "$expected"
                echo "---"
            } >> "$FAILURES_LOG"
            fail=$((fail + 1))
            continue
        fi
        
        if [[ "$expected" == "$actual" ]]; then
            [[ "$SUMMARY_ONLY" != "true" ]] && echo "PASS $name"
            pass=$((pass + 1))
        else
            [[ "$SUMMARY_ONLY" != "true" ]] && echo "FAIL $name (output mismatch)"
            {
                echo "=== $name ==="
                echo "expected:"
                echo "$expected"
                echo "---"
                echo "actual:"
                echo "$actual"
                echo "---"
                diff <(echo "$expected") <(echo "$actual") || true
                echo
            } >> "$FAILURES_LOG"
            fail=$((fail + 1))
        fi
    else
        # No zsh available - just check zshrs runs without crashing
        if timeout "$TIMEOUT" "$ZSHRS" "$zsh_file" >/dev/null 2>&1; then
            [[ "$SUMMARY_ONLY" != "true" ]] && echo "PASS $name (syntax only)"
            pass=$((pass + 1))
        else
            exit_code=$?
            if [[ $exit_code -eq 124 ]]; then
                [[ "$SUMMARY_ONLY" != "true" ]] && echo "FAIL $name (timeout)"
            else
                [[ "$SUMMARY_ONLY" != "true" ]] && echo "FAIL $name (exit $exit_code)"
            fi
            {
                echo "=== $name ==="
                echo "zshrs exit: $exit_code"
                echo "---"
            } >> "$FAILURES_LOG"
            fail=$((fail + 1))
        fi
    fi
done

echo
echo "Results: $pass passed, $fail failed, $skip skipped (total: $total)"
[[ $HAS_ZSH == "true" ]] || echo "(no zsh found - syntax-only mode)"

if [[ $fail -gt 0 ]]; then
    echo "Failures logged to: $FAILURES_LOG"
    exit 1
fi
