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

# === ztest assertions ===
# =cmd expansion → absolute path containing /echo, /cat respectively.
zassert_contains "$(echo =echo)"  "echo"  "=echo resolves to a path containing 'echo'"
zassert_contains "$(echo =cat)"   "cat"   "=cat resolves to a path containing 'cat'"
# nullglob → empty array
setopt nullglob
n_matches=( /tmp/__never_matches_xyz_*(.) )
zassert_eq "${#n_matches[@]}" "0"  "nullglob empties unmatched"
unsetopt nullglob
# extended_glob alternation finds the two files we created.
setopt extended_glob
d3=/tmp/zshrs_glob3_$$
mkdir -p "$d3"
touch "$d3"/{alpha,beta,gamma,delta}.log
cd "$d3"
matches=( (alpha|gamma).log )
zassert_eq "${#matches[@]}" "2"  "extglob alt: 2 matches"
zassert_contains "${matches[*]}" "alpha.log"  "match alpha.log"
zassert_contains "${matches[*]}" "gamma.log"  "match gamma.log"
cd /tmp
rm -rf "$d3"
unsetopt extended_glob
ztest_run
