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
        # No zsh available - just check zshrs runs without crashing.
        # Some corpus files are oh-my-zsh-style plugins that intentionally
        # `return N` / `exit N` when their host tool is missing, or are zsh
        # test drivers that bail without a script-file argument. Any
        # non-zero exit from those is expected — what we're really pinning
        # in syntax-only mode is that zshrs doesn't timeout, segfault, or
        # panic on the input. Each entry below was verified to exit
        # non-zero for a reason zshrs shares with real zsh (not a zshrs
        # bug): missing sourced sibling files, zunit-DSL files real `zsh -n`
        # also parse-errors on, or a standard NOMATCH glob bailout. The
        # 134/139 guard below still fails any real SIGABRT/SIGSEGV.
        case "$name" in
            thefuck.plugin.zsh|zsh-z.plugin.zsh|ztst.zsh) source_only=1 ;;
            # `source` of absent music/spotify sibling files (exit 127).
            macos.plugin.zsh) source_only=1 ;;
            # zunit-DSL (@setup/@test blocks) — `zsh -n` also parse-errors.
            zpwr-syntax.zsh|zpwr-verbs.zsh) source_only=1 ;;
            # NOMATCH glob bailout matching real zsh's default.
            zsh-autocomplete.plugin.zsh) source_only=1 ;;
            # `add-zsh-hook` autoload with no zsh fn dir in fpath — real
            # zsh also errors "function definition file not found".
            alias-finder.plugin.zsh) source_only=1 ;;
            *) source_only=0 ;;
        esac
        if timeout "$TIMEOUT" "$ZSHRS" "$zsh_file" >/dev/null 2>&1; then
            [[ "$SUMMARY_ONLY" != "true" ]] && echo "PASS $name (syntax only)"
            pass=$((pass + 1))
        else
            exit_code=$?
            # 124 = timeout, 134/139 = SIGABRT/SIGSEGV — always real failures.
            if [[ $exit_code -eq 124 ]]; then
                [[ "$SUMMARY_ONLY" != "true" ]] && echo "FAIL $name (timeout)"
            elif [[ $source_only -eq 1 && $exit_code -ne 134 && $exit_code -ne 139 ]]; then
                [[ "$SUMMARY_ONLY" != "true" ]] && echo "PASS $name (source-only bailout, exit $exit_code)"
                pass=$((pass + 1))
                continue
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
