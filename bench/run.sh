#!/usr/bin/env bash
#
# Cross-shell benchmark harness: zshrs vs zsh vs fish vs nushell (plus bash
# when present). Each workload is expressed once per dialect (POSIX-ish for
# zshrs/zsh/bash, fish, nushell) so every shell runs its own native idiom for
# the same logical work. Runs hyperfine, writes a Markdown table per workload
# to bench/results.md.
#
# Usage:
#   bench/run.sh                         # all shells found, all workloads
#   bench/run.sh --workload loop         # one workload (repeatable)
#   bench/run.sh --warmup 5 --runs 20
#   bench/run.sh --shells "zshrs zsh fish nu"
#   ZSHRS_BIN=/path/to/zshrs bench/run.sh
#
# ZSHRS_BIN defaults to target/release/zshrs. Debug builds are never benchmarked.
#
# Pure measurement: no pass/fail adjudication. A snippet that exits non-zero
# aborts hyperfine, so a broken workload cannot produce a number.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RESULTS="${REPO_ROOT}/bench/results.md"
WARMUP=3
RUNS=10
SHELL_NAMES=(zshrs zsh fish nu bash)
ONLY=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --warmup)   WARMUP="$2"; shift 2 ;;
        --runs)     RUNS="$2"; shift 2 ;;
        --shells)   read -r -a SHELL_NAMES <<< "$2"; shift 2 ;;
        --workload) ONLY+=("$2"); shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

command -v hyperfine >/dev/null 2>&1 || { echo "hyperfine missing — brew install hyperfine" >&2; exit 1; }

ZSHRS_BIN="${ZSHRS_BIN:-${REPO_ROOT}/target/release/zshrs}"
[[ -x "${ZSHRS_BIN}" ]] || { echo "no release binary at ${ZSHRS_BIN} — build the release binary first" >&2; exit 1; }

# Per-shell: executable, and the flags that skip user config so the numbers
# measure the interpreter, not the benchmarker's dotfiles.
declare -A EXE FLAGS DIALECT
EXE[zshrs]="${ZSHRS_BIN:-}";       FLAGS[zshrs]="-f -c"
EXE[zsh]="$(command -v zsh || true)";   FLAGS[zsh]="-f -c"
EXE[bash]="$(command -v bash || true)"; FLAGS[bash]="--norc --noprofile -c"
EXE[fish]="$(command -v fish || true)"; FLAGS[fish]="--no-config -c"
EXE[nu]="$(command -v nu || true)";     FLAGS[nu]="--no-config-file -c"
DIALECT[zshrs]=posix; DIALECT[zsh]=posix; DIALECT[bash]=posix
DIALECT[fish]=fish;   DIALECT[nu]=nu

ACTIVE=()
for s in "${SHELL_NAMES[@]}"; do
    if [[ -n "${EXE[$s]:-}" && -x "${EXE[$s]}" ]]; then
        ACTIVE+=("$s")
    else
        echo "skipping ${s}: not found" >&2
    fi
done
[[ ${#ACTIVE[@]} -ge 2 ]] || { echo "need at least two shells" >&2; exit 1; }

# Workloads: name|description. Snippets live in snippet() below; none may
# contain a single quote (they are single-quoted into the hyperfine command).
WORKLOADS=(
    "startup|Startup (no-op)"
    "loop|Arithmetic loop (100000 iterations)"
    "func|Function calls (20000)"
    "strcat|String append (5000)"
    "cmdsubst|Command substitution (500)"
    "spawn|External command spawn (200 x /usr/bin/true)"
    "pipeline|Pipeline (seq 200000 | sort -n | uniq | wc -l)"
    "glob|Glob (src/*/*.rs)"
)

snippet() {
    local dialect="$1" name="$2"
    case "${dialect}:${name}" in
        posix:startup)  echo ':' ;;
        posix:loop)     echo 'i=0; while [ $i -lt 100000 ]; do i=$((i+1)); done' ;;
        posix:func)     echo 'f() { :; }; i=0; while [ $i -lt 20000 ]; do f; i=$((i+1)); done' ;;
        posix:strcat)   echo 's=; i=0; while [ $i -lt 5000 ]; do s=$s$i; i=$((i+1)); done' ;;
        posix:cmdsubst) echo 'i=0; while [ $i -lt 500 ]; do x=$(echo $i); i=$((i+1)); done' ;;
        posix:spawn)    echo 'i=0; while [ $i -lt 200 ]; do /usr/bin/true; i=$((i+1)); done' ;;
        posix:pipeline) echo 'seq 1 200000 | sort -n | uniq | wc -l >/dev/null' ;;
        posix:glob)     echo "cd ${REPO_ROOT} && echo src/*/*.rs >/dev/null" ;;

        fish:startup)   echo 'true' ;;
        fish:loop)      echo 'set i 0; while test $i -lt 100000; set i (math $i + 1); end' ;;
        fish:func)      echo 'function f; end; set i 0; while test $i -lt 20000; f; set i (math $i + 1); end' ;;
        fish:strcat)    echo 'set s ""; set i 0; while test $i -lt 5000; set s $s$i; set i (math $i + 1); end' ;;
        fish:cmdsubst)  echo 'set i 0; while test $i -lt 500; set x (echo $i); set i (math $i + 1); end' ;;
        fish:spawn)     echo 'set i 0; while test $i -lt 200; /usr/bin/true; set i (math $i + 1); end' ;;
        fish:pipeline)  echo 'seq 1 200000 | sort -n | uniq | wc -l >/dev/null' ;;
        fish:glob)      echo "cd ${REPO_ROOT}; and echo src/*/*.rs >/dev/null" ;;

        nu:startup)     echo 'null' ;;
        nu:loop)        echo 'mut i = 0; while $i < 100000 { $i += 1 }' ;;
        nu:func)        echo 'def f [] {}; mut i = 0; while $i < 20000 { f; $i += 1 }' ;;
        nu:strcat)      echo 'mut s = ""; mut i = 0; while $i < 5000 { $s = $s + ($i | into string); $i += 1 }' ;;
        nu:cmdsubst)    echo 'mut i = 0; while $i < 500 { let x = (^echo $i); $i += 1 }' ;;
        nu:spawn)       echo 'mut i = 0; while $i < 200 { ^/usr/bin/true; $i += 1 }' ;;
        nu:pipeline)    echo '^seq 1 200000 | ^sort -n | ^uniq | ^wc -l | ignore' ;;
        nu:glob)        echo "cd ${REPO_ROOT}; glob src/*/*.rs | ignore" ;;
        *) echo "no snippet for ${dialect}:${name}" >&2; return 1 ;;
    esac
}

wanted() {
    [[ ${#ONLY[@]} -eq 0 ]] && return 0
    local w
    for w in "${ONLY[@]}"; do [[ "$w" == "$1" ]] && return 0; done
    return 1
}

{
    echo "# Shell benchmark results"
    echo
    echo "Generated $(date -u '+%Y-%m-%d %H:%M:%S UTC') on $(uname -srm); hyperfine --warmup ${WARMUP} --runs ${RUNS}"
    echo
    echo "| Shell | Binary | Version |"
    echo "|---|---|---|"
    for s in "${ACTIVE[@]}"; do
        echo "| ${s} | \`${EXE[$s]}\` | $("${EXE[$s]}" --version 2>&1 | head -1) |"
    done
    echo
} > "${RESULTS}"

for entry in "${WORKLOADS[@]}"; do
    name="${entry%%|*}"
    desc="${entry#*|}"
    wanted "$name" || continue
    cmds=()
    names=()
    for s in "${ACTIVE[@]}"; do
        cmds+=("${EXE[$s]} ${FLAGS[$s]} '$(snippet "${DIALECT[$s]}" "$name")'")
        names+=(-n "$s")
    done
    echo "## ${desc}" >> "${RESULTS}"
    echo >> "${RESULTS}"
    md="$(mktemp)"
    hyperfine --shell=none --warmup "${WARMUP}" --runs "${RUNS}" \
        --export-markdown "${md}" "${names[@]}" "${cmds[@]}" >/dev/null 2>&1
    cat "${md}" >> "${RESULTS}"
    rm -f "${md}"
    echo >> "${RESULTS}"
done

echo "Wrote ${RESULTS}"
