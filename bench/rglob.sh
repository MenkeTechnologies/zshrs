#!/usr/bin/env bash
#
# Recursive-glob benchmark: zsh vs zshrs (parallel descent) vs zshrs (serial
# descent, `[glob] recursive_parallel = false`). Runs hyperfine over a generated
# directory tree and writes a Markdown table per pattern to
# bench/results_rglob.md.
#
# Only zsh and zshrs are compared: `**/` is zsh syntax, and bash/ksh/dash/fish/nu
# either lack it or give it different semantics, so a cross-shell number would
# not measure the same work.
#
# Before timing, every shell's sorted output is compared against zsh's. A
# mismatch aborts, so a wrong-but-fast result cannot produce a number.
#
# Usage:
#   bench/rglob.sh
#   bench/rglob.sh --warmup 5 --runs 30
#   ZSHRS_BIN=/path/to/zshrs bench/rglob.sh
#
# ZSHRS_BIN defaults to target/release/zshrs. Debug builds are never benchmarked.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RESULTS="${REPO_ROOT}/bench/results_rglob.md"
WARMUP=3
RUNS=15

# Tree shape: TOP_DIRS x SUB_DIRS directories, FILES_PER_DIR files in each leaf.
TOP_DIRS=100
SUB_DIRS=10
FILES_PER_DIR=30

while [[ $# -gt 0 ]]; do
    case "$1" in
        --warmup) WARMUP="$2"; shift 2 ;;
        --runs)   RUNS="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

command -v hyperfine >/dev/null 2>&1 || { echo "hyperfine missing — brew install hyperfine" >&2; exit 1; }

ZSHRS_BIN="${ZSHRS_BIN:-${REPO_ROOT}/target/release/zshrs}"
[[ -x "${ZSHRS_BIN}" ]] || { echo "no release binary at ${ZSHRS_BIN} — build the release binary first" >&2; exit 1; }
ZSH_BIN="$(command -v zsh || true)"
[[ -n "${ZSH_BIN}" ]] || { echo "zsh not found" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
TREE="${WORK}/tree"
HOME_PAR="${WORK}/home-parallel"
HOME_SER="${WORK}/home-serial"
mkdir -p "${HOME_PAR}" "${HOME_SER}"
printf '[glob]\nrecursive_parallel = true\n'  > "${HOME_PAR}/zshrs.toml"
printf '[glob]\nrecursive_parallel = false\n' > "${HOME_SER}/zshrs.toml"

# Extensions cycle rs/txt/md so `**/*.rs` selects a third of the files.
perl -e '
    use File::Path qw(make_path);
    my ($root, $top, $sub, $files) = @ARGV;
    my @ext = qw(rs txt md);
    for my $a (0 .. $top - 1) {
        for my $b (0 .. $sub - 1) {
            my $d = "$root/d$a/s$b";
            make_path($d);
            for my $f (0 .. $files - 1) {
                open my $h, ">", "$d/f$f.$ext[$f % 3]" or die "$d: $!";
                close $h;
            }
        }
    }
' "${TREE}" "${TOP_DIRS}" "${SUB_DIRS}" "${FILES_PER_DIR}"

FILE_COUNT="$(find "${TREE}" -type f | wc -l | tr -d ' ')"
DIR_COUNT="$(find "${TREE}" -mindepth 1 -type d | wc -l | tr -d ' ')"
CORES="$(sysctl -n hw.ncpu 2>/dev/null || nproc)"

# name|description|glob. Globs may not contain a single quote.
PATTERNS=(
    'all|`**/*` (every file and directory)|**/*'
    'ext|`**/*.rs` (extension filter)|**/*.rs'
    'qual|`**/*(.)` (plain files, qualifier)|**/*(.)'
)

zshrs_cmd() {
    local home="$1" glob="$2"
    echo "env ZSHRS_HOME=${home} ${ZSHRS_BIN} -f -c 'cd ${TREE}; print -rl -- ${glob} >/dev/null'"
}

zsh_cmd() {
    local glob="$1"
    echo "${ZSH_BIN} -f -c 'cd ${TREE}; print -rl -- ${glob} >/dev/null'"
}

digest() {
    (cd "${TREE}" && "$@" | LC_ALL=C sort | cksum)
}

{
    echo "# Recursive glob benchmark results"
    echo
    echo "Generated $(date -u '+%Y-%m-%d %H:%M:%S UTC') on $(uname -srm), ${CORES} logical cores; hyperfine --warmup ${WARMUP} --runs ${RUNS}"
    echo
    echo "Tree: ${FILE_COUNT} files in ${DIR_COUNT} directories (${TOP_DIRS} x ${SUB_DIRS} leaf directories, ${FILES_PER_DIR} files each)."
    echo
    echo "| Shell | Binary | Version |"
    echo "|---|---|---|"
    echo "| zsh | \`${ZSH_BIN}\` | $("${ZSH_BIN}" --version | head -1) |"
    echo "| zshrs | \`${ZSHRS_BIN}\` | $("${ZSHRS_BIN}" --version | head -1) |"
    echo
} > "${RESULTS}"

for entry in "${PATTERNS[@]}"; do
    IFS='|' read -r name desc glob <<< "${entry}"

    want="$(digest "${ZSH_BIN}" -f -c "print -rl -- ${glob}")"
    for home in "${HOME_PAR}" "${HOME_SER}"; do
        got="$(ZSHRS_HOME="${home}" digest "${ZSHRS_BIN}" -f -c "print -rl -- ${glob}")"
        if [[ "${got}" != "${want}" ]]; then
            echo "output mismatch vs zsh for ${glob} (ZSHRS_HOME=${home})" >&2
            exit 1
        fi
    done

    echo "## ${desc}" >> "${RESULTS}"
    echo >> "${RESULTS}"
    md="$(mktemp)"
    hyperfine --shell=none --warmup "${WARMUP}" --runs "${RUNS}" \
        --export-markdown "${md}" \
        -n "zsh" "$(zsh_cmd "${glob}")" \
        -n "zshrs (parallel)" "$(zshrs_cmd "${HOME_PAR}" "${glob}")" \
        -n "zshrs (serial)" "$(zshrs_cmd "${HOME_SER}" "${glob}")" >/dev/null 2>&1
    cat "${md}" >> "${RESULTS}"
    rm -f "${md}"
    echo >> "${RESULTS}"
done

echo "Wrote ${RESULTS}"
