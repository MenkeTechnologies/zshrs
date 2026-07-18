#!/usr/bin/env bash
# bp — bump version, commit, tag, push, publish.
#
# Usage:  scripts/bp.sh <NEW-VERSION>
# Example: scripts/bp.sh 0.10.8
#
# Bumps the version in:
#   - root Cargo.toml: [workspace.package].version + [workspace.dependencies]
#     (members inherit via `version.workspace = true` / `xxx.workspace = true`);
#   - the docs build-lines (docs/index.html, docs/reference.html) and the
#     man .TH lines (man/man1/zshrs.1, zshrsall.1) — REQUIRED so the meta-repo
#     version-sync gates (docs build-line + man .TH must match Cargo) stay green.
#
# Publishes zshrs-daemon then zshrs (dependency order; zshrs depends on the
# daemon). Crucially it publishes from a CLEAN git worktree of the new tag, NOT
# the live working tree — this repo is edited by many concurrent sessions, so the
# working tree is usually dirty, and `cargo publish` packages the working
# directory (which would refuse on a dirty tree, or bake unrelated WIP into the
# published crate permanently).

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

# Bump the version stamped into the docs build-lines + man pages, so the
# meta-repo version-sync gates stay green. The docs carry a `v` prefix
# (`zshrs vX.Y.Z`) so the digit isn't on a word boundary — match `v` explicitly.
# `\Q…\E` quotes the dots in $OLD.
echo "→ bumping docs build-lines + man .TH  ($OLD → $NEW)"
for f in docs/index.html docs/reference.html; do
    [[ -f "$f" ]] && perl -pi -e "s/zshrs v\Q$OLD\E/zshrs v$NEW/g" "$f"
done
TODAY=$(date +%Y-%m-%d)
for f in man/man1/zshrs.1 man/man1/zshrsall.1; do
    [[ -f "$f" ]] || continue
    perl -pi -e "s/\"zshrs \Q$OLD\E\"/\"zshrs $NEW\"/g" "$f"          # .TH version
    perl -pi -e "s/(^\.TH \S+ 1 )\"[0-9-]+\"/\${1}\"$TODAY\"/" "$f"   # .TH date
done

# Build + lex parity gate before tagging anything.
echo "→ cargo build (sanity check)"
cargo build --quiet
# corpus_lexer_parity is a module of the aggregated `parity` test
# binary (tests/parity/lexer_parity.rs), not a standalone test target.
echo "→ cargo test --test parity corpus_lexer_parity (sanity check)"
cargo test --quiet --test parity corpus_lexer_parity > /dev/null

# Stage every bumped file (docs/ is gitignored here, so force-add the already
# tracked docs files), commit, tag, push the commit + the new tag. We push only
# the new tag, not `--tags` (which also pushes every stale local tag and fails
# if any conflicts with the remote).
git add Cargo.toml Cargo.lock man/man1/zshrs.1 man/man1/zshrsall.1 2>/dev/null || true
git add -f docs/index.html docs/reference.html 2>/dev/null || true
git commit -m "bump v$NEW"
git tag "v$NEW"
git push
git push origin "v$NEW"

# Publish from a CLEAN worktree of the new tag (see header). --no-verify skips
# cargo's redundant cold re-compile: the `cargo build` sanity gate above already
# proved the workspace builds, and the package is that same source minus the
# excluded tests/docs. cargo itself waits for each crate to index before
# returning, so zshrs resolves the just-published zshrs-daemon.
echo "→ publishing from a clean worktree of v$NEW"
WORKTREE="$(mktemp -d)/zshrs-publish"
git worktree add --detach "$WORKTREE" "v$NEW" >/dev/null
cleanup() {
    git worktree remove --force "$WORKTREE" 2>/dev/null || true
    git worktree prune 2>/dev/null || true
}
trap cleanup EXIT

publish() {
    echo "→ cargo publish -p $1 --no-verify"
    ( cd "$WORKTREE" && cargo publish -p "$1" --no-verify )
}

# zshrs-parse + compsys absorbed into runtime — no longer published separately.
# Dependency order: znative (leaf C-ABI plugin SDK) and zshrs-daemon are both
# workspace deps of zshrs, so both MUST publish first — otherwise `cargo publish
# -p zshrs` fails to resolve `znative = "^$NEW"` / the daemon at the new version
# (they only exist at the previous version on crates.io until published here).
publish znative
publish zshrs-daemon
publish zshrs

echo "✓ bumped to v$NEW, pushed, and published to crates.io"
