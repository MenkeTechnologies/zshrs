#!/usr/bin/env bash
# For every tests/*.sh tracked in git across the umbrella, pin that
# the file has the executable bit set.
#
# Test scripts in the meta repo's `tests/` dir are invoked by CI as
# `bash tests/<name>.sh` (which works regardless of mode), but they
# are also routinely invoked by humans during development as
# `./tests/<name>.sh` or by neighbour scripts via `$0` relative
# paths. A script without +x exits with `Permission denied` on
# direct invocation — confusing to anyone who clones the repo and
# tries the canonical "run the gate locally" workflow.
#
# Additionally: when these scripts are vendored INTO submodules
# (as the docs-* gates were in iter-50), the submodule's own ci.yml
# may invoke them as `./tests/...` rather than `bash tests/...`
# depending on the author's style. +x is the universal floor that
# works in both modes.
#
# Test scans `git ls-files` for `tests/*.sh` paths at depth 1 ONLY
# (i.e., top-level `tests/foo.sh`, NOT nested `tests/corpus/foo.sh`
# or `tests/data/foo.sh`). Nested paths conventionally hold
# fixtures / corpora / vendored test data — `.sh` files there are
# inputs to the test runner, not test scripts themselves. zshrs's
# `tests/lexer_corpus/*.sh` is the canonical example: bare command
# snippets fed to the lexer's parity harness. Querying git index
# mode (100755 = executable, 100644 = not), the test fails on any
# top-level `tests/*.sh` whose mode is non-executable.
set -uo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root" || exit
ok=1

checked=0
flagged=0
files_with_issues=()

# `git ls-files -s` prints `<mode> <sha> <stage>\t<path>` per file.
# We filter to tests/*.sh and parse the mode field.
while IFS=$'\t' read -r meta path; do
    mode=$(echo "$meta" | awk '{print $1}')
    [[ "$path" == *.sh ]] || continue
    # Depth-1 only: tests/foo.sh or <submodule>/tests/foo.sh.
    # Excludes tests/<subdir>/foo.sh (fixtures / corpora).
    [[ "$path" =~ ^tests/[^/]+\.sh$ || "$path" =~ ^[^/]+/tests/[^/]+\.sh$ ]] || continue
    checked=$((checked + 1))
    if [[ "$mode" != "100755" ]]; then
        echo "FAIL  $path: git mode=$mode (expected 100755 / executable)"
        flagged=$((flagged + 1))
        files_with_issues+=("$path")
        ok=0
    fi
done < <(git ls-files -s 2>/dev/null)

echo "---"
echo "Summary: $checked tests/*.sh scripts checked, $flagged without +x in git index"

[[ $ok -eq 1 ]] && exit 0 || exit 1
