#!/usr/bin/env bash
#
# Phase M1 bench harness — scaffolded by session 2026-04-27.
#
# Measures zshrs against zsh and bash on the canonical workloads in the
# ROADMAP (cold start, warm start, pipeline, builtin tightloop, glob, compinit).
# Runs hyperfine, writes Markdown table to bench/results.md.
#
# Usage:
#   cargo build --release        # use the optimized binary, not debug
#   bench/run.sh                 # measures against /bin/zsh + /bin/bash
#   bench/run.sh --warmup 5      # extra warmup runs
#   bench/run.sh --shells "/usr/local/bin/zsh /opt/homebrew/bin/zsh"
#
# Output: bench/results.md (Markdown table, ready to commit).
#
# This is a PURE measurement script. It does not adjudicate pass/fail — that's
# the maintainer's call after reading the numbers. Phase M2 publishes the
# numbers; Phase M3 wires this into CI as a regression alarm.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ZSHRS_BIN="${REPO_ROOT}/target/release/zshrs"

WARMUP=3
RUNS=10
SHELLS=("${ZSHRS_BIN}" "/bin/zsh" "/bin/bash")
RESULTS="${REPO_ROOT}/bench/results.md"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --warmup) WARMUP="$2"; shift 2 ;;
        --runs)   RUNS="$2"; shift 2 ;;
        --shells) read -r -a SHELLS <<< "$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

if [[ ! -x "${ZSHRS_BIN}" ]]; then
    echo "no release binary at ${ZSHRS_BIN} — run \`cargo build --release\` first" >&2
    exit 1
fi
if ! command -v hyperfine >/dev/null 2>&1; then
    echo "hyperfine missing — install via \`brew install hyperfine\` or \`cargo install hyperfine\`" >&2
    exit 1
fi

bench_one() {
    local label="$1"
    local cmd="$2"
    local args=()
    for s in "${SHELLS[@]}"; do
        args+=("${s} -c '${cmd}'")
    done
    echo "## ${label}" >> "${RESULTS}"
    echo >> "${RESULTS}"
    hyperfine \
        --warmup "${WARMUP}" \
        --runs "${RUNS}" \
        --export-markdown - \
        "${args[@]}" \
        >> "${RESULTS}" 2>/dev/null
    echo >> "${RESULTS}"
}

: > "${RESULTS}"
{
    echo "# zshrs bench results"
    echo
    echo "Generated $(date -u '+%Y-%m-%d %H:%M:%S UTC') on $(uname -srm)"
    echo
    echo "Shells: ${SHELLS[*]}"
    echo
} >> "${RESULTS}"

bench_one "Cold start (true)"           "true"
bench_one "Tight loop (1000 echos)"     'i=0; while [ $i -lt 1000 ]; do echo $i >/dev/null; i=$((i+1)); done'
bench_one "Pipeline (seq | tr | sort | uniq | wc)" \
    "echo \$(seq 1 100) | tr ' ' '\\n' | sort | uniq | wc -l"
bench_one "Glob (**/*.rs in repo)" \
    "cd ${REPO_ROOT} && echo **/*.rs >/dev/null"

echo
echo "Wrote ${RESULTS}"
echo
echo "Next: review results.md, identify regressions, file each as Phase M followup."
