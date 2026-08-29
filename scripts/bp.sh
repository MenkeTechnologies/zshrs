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
# Publishes znative, then zshrs-daemon, then zshrs (dependency order; zshrs
# depends on both), and finally `zsh` — the alias crate: the same source under a
# second crates.io name. Crucially it publishes from a CLEAN git worktree of the
# new tag, NOT
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
# Second worktree of the same tag, for the `zsh` alias crate below. Separate so
# the two-line rename it needs never contaminates the tarball zshrs ships from.
WORKTREE_ZSH="$(mktemp -d)/zsh-publish"
git worktree add --detach "$WORKTREE_ZSH" "v$NEW" >/dev/null
cleanup() {
    git worktree remove --force "$WORKTREE" 2>/dev/null || true
    git worktree remove --force "$WORKTREE_ZSH" 2>/dev/null || true
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

# `zsh` — the alias crate. Same source, same version, published under the name
# the shell answers to (and the name of this crate's [lib]). It is a separate
# crates.io registration, so cargo has to see a manifest whose [package] name IS
# `zsh`; there is no rename flag for publish. Two lines change, in a throwaway
# worktree of the tag:
#
#   [package] name                zshrs -> zsh
#   [workspace.dependencies]      zshrs = { path = "." } gains package = "zsh",
#                                 without which runtime/'s `zshrs.workspace =
#                                 true` stops resolving — path "." now provides
#                                 a crate called `zsh`.
#
# --allow-dirty is required by exactly those two edits; the worktree is
# otherwise an untouched checkout of v$NEW. Publishing zsh LAST means a failure
# here leaves the three real crates already up, and re-running only needs this
# step. Skipping it is what let `zsh` fall 19 patches behind before v0.12.44.
echo "→ publishing the zsh alias crate from a clean worktree of v$NEW"
python3 - "$WORKTREE_ZSH/Cargo.toml" "$NEW" <<'PYZSH'
import sys

path, new = sys.argv[1], sys.argv[2]
src = open(path).read()

pkg_old = '[package]\nname = "zshrs"\n'
pkg_new = '[package]\nname = "zsh"\n'
if pkg_old not in src:
    sys.exit('bp: [package] name = "zshrs" not found at the top of Cargo.toml')
src = src.replace(pkg_old, pkg_new, 1)

dep_old = 'zshrs = { path = ".", version = "%s" }' % new
dep_new = 'zshrs = { path = ".", version = "%s", package = "zsh" }' % new
if dep_old not in src:
    sys.exit("bp: [workspace.dependencies] zshrs line for %s not found" % new)
src = src.replace(dep_old, dep_new, 1)

open(path, "w").write(src)
PYZSH
( cd "$WORKTREE_ZSH" && cargo publish -p zsh --no-verify --allow-dirty )

echo "✓ bumped to v$NEW, pushed, and published to crates.io"
echo "  znative $NEW · zshrs-daemon $NEW · zshrs $NEW · zsh $NEW"
