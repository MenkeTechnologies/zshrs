#!/usr/bin/env zshrs
# GLOB_SUBST + GLOB_COMPLETE + ~tilde + =cmd expansion.
# Ported from Src/glob.c (glob processing) + Src/subst.c (=cmd path).

echo "── tilde expansion ──"
echo "HOME via ~: ~"
echo "explicit ~/$USER does not double-expand: ~/$USER"
echo "named: ~root"  # may or may not exist

echo "── =cmd resolution (zsh-specific) ──"
# =ls → /bin/ls
echo "=echo → $(echo =echo)"
echo "=cat → $(echo =cat)"

echo "── \$param as glob source needs setopt globsubst ──"
pat="*.txt"
tmpdir=/tmp/zshrs_glob_$$
mkdir -p "$tmpdir"
touch "$tmpdir/a.txt" "$tmpdir/b.txt" "$tmpdir/c.log"
cd "$tmpdir"

echo "without globsubst (literal pat):"
unsetopt globsubst
for f in $pat; do echo "  $f"; done

echo "with globsubst (expand pat):"
setopt globsubst
for f in $pat; do echo "  $f"; done
unsetopt globsubst

cd /tmp
rm -rf "$tmpdir"

echo "── nullglob: empty match → no args ──"
setopt nullglob
matches=( /tmp/__never_matches_xyz_*(.) )
echo "matches: ${#matches[@]} (expected 0)"
unsetopt nullglob

echo "── failglob: empty match → error ──"
(
    setopt failglob
    # Wrap in subshell so the error doesn't terminate this script.
    ls /tmp/__never_matches_xyz_* 2>&1 | head -1
) 2>&1 | head -3

echo "── extended glob alternation ──"
setopt extended_glob
tmpdir=/tmp/zshrs_glob2_$$
mkdir -p "$tmpdir"
touch "$tmpdir"/{alpha,beta,gamma,delta}.log
cd "$tmpdir"
# (alpha|gamma) — match either.
print -l (alpha|gamma).log
cd /tmp
rm -rf "$tmpdir"
unsetopt extended_glob
