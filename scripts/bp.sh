#!/usr/bin/env bash
# bp — bump version, commit, tag, push, publish all 4 workspace crates.
#
# Usage:  scripts/bp.sh <NEW-VERSION>
# Example: scripts/bp.sh 0.10.8
#
# Touches exactly one file (root Cargo.toml). The workspace.package
# version + the [workspace.dependencies] block both get the new version
# in one sed pass. All four members (parse, compsys, daemon, root)
# inherit via `version.workspace = true` and `xxx.workspace = true`.
#
# Publish order is dependency-driven:
# (zshrs-parse absorbed into runtime; no separate publish)
#   2. compsys      (no internal deps)
#   3. zshrs-daemon
#   4. zshrs        (depends on all three)
#
# Each `cargo publish` waits a few seconds for the previous to land on
# crates.io before the next one resolves it.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: scripts/bp.sh <NEW-VERSION>" >&2
    echo "  e.g. scripts/bp.sh 0.10.8" >&2
    exit 1
fi

NEW=$1
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

# Validate semver-ish (X.Y.Z, optionally with pre-release)
if [[ ! "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$ ]]; then
    echo "error: '$NEW' doesn't look like X.Y.Z" >&2
    exit 1
fi

OLD=$(awk '/^\[workspace\.package\]/{f=1} f && /^version *=/{gsub(/"/,"",$3); print $3; exit}' Cargo.toml)
if [[ -z "$OLD" ]]; then
    echo "error: couldn't find existing [workspace.package] version in Cargo.toml" >&2
    exit 1
fi
if [[ "$OLD" == "$NEW" ]]; then
    echo "error: $NEW is already the current version" >&2
    exit 1
fi

echo "→ $OLD  →  $NEW"

# Bump [workspace.package] version + every [workspace.dependencies] entry.
# All four version literals live in this single block; no other Cargo.toml
# in the tree carries a literal version (members use `*.workspace = true`).
python3 - <<PY
import re, sys
path = "Cargo.toml"
src = open(path).read()
old = "$OLD"
new = "$NEW"

# 1) [workspace.package] version
src = re.sub(
    r'(\[workspace\.package\][^\[]*?\nversion *= *")[^"]+(")',
    rf'\g<1>{new}\g<2>',
    src,
    count=1,
    flags=re.DOTALL,
)

# 2) Every literal version inside [workspace.dependencies] block
def fix_block(m):
    block = m.group(0)
    fixed = re.sub(r'(version *= *")[^"]+(")', rf'\g<1>{new}\g<2>', block)
    return fixed

src = re.sub(
    r'\[workspace\.dependencies\][^\[]*',
    fix_block,
    src,
    count=1,
    flags=re.DOTALL,
)

open(path, 'w').write(src)
PY

# Sanity check: every "version" literal in root Cargo.toml is now $NEW.
if grep -E 'version *= *"' Cargo.toml | grep -v "\"$NEW\"" | grep -v '\.workspace *= *true' | head; then
    echo "warning: some version literals didn't update — review Cargo.toml" >&2
fi

# Build + lex parity gate before tagging anything.
echo "→ cargo build (sanity check)"
cargo build --quiet
# corpus_lexer_parity is a module of the aggregated `parity` test
# binary (tests/parity/lexer_parity.rs), not a standalone test target.
echo "→ cargo test --test parity corpus_lexer_parity (sanity check)"
cargo test --quiet --test parity corpus_lexer_parity > /dev/null

# Stage, commit, tag, push.
git add Cargo.toml Cargo.lock 2>/dev/null || true
git commit -m "bump v$NEW"
git tag "v$NEW"
git push
git push --tags

# Publish in dependency order, waiting a few seconds between each so the
# next resolve sees the previous on crates.io.
publish() {
    echo "→ cargo publish -p $1"
    cargo publish -p "$1"
    echo "→ sleeping 8s for crates.io to index $1@$NEW"
    sleep 8
}

# zshrs-parse + compsys absorbed into runtime — no longer published
# separately. Only zshrs-daemon and zshrs remain as standalone crates.
publish zshrs-daemon
publish zshrs

echo "✓ bumped to v$NEW and published all crates"
