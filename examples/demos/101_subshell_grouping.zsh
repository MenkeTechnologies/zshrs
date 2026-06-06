#!/usr/bin/env zshrs
# Subshell vs group — ( ) vs { } scope rules.
# Ported from Src/exec.c execlist / execsubsh.

echo "── ( ) subshell — var assignments don't escape ──"
x=outer
( x=inner; echo "inside subshell: x=$x" )
echo "outside: x=$x"

echo "── { } group — same scope as parent ──"
y=outer
{ y=inner; echo "inside group: y=$y"; }
echo "outside: y=$y"

echo "── ( ) for env-only changes ──"
(
    cd /tmp
    echo "subshell pwd: $PWD"
)
echo "back in: $PWD"

echo "── nested subshells ──"
z=outer
(
    z=middle
    (
        z=innermost
        echo "deepest: $z"
    )
    echo "middle: $z"
)
echo "outer: $z"

echo "── { } block for output collection ──"
{
    echo line1
    echo line2
    echo line3
} | wc -l

echo "── { } group with redirection ──"
{ echo a; echo b; echo c; } > /tmp/zshrs_grp_$$
wc -l < /tmp/zshrs_grp_$$
rm -f /tmp/zshrs_grp_$$

echo "── ( ) cannot pollute parent flags ──"
( setopt nullglob; [[ -o nullglob ]] && echo "subshell nullglob ON"; )
[[ -o nullglob ]] && echo "outer nullglob ON" || echo "outer nullglob OFF"

echo "── exit code of subshell ──"
( exit 7 )
echo "subshell exit: $?"

echo "── subshell as command sub ──"
result=$( echo "subshell output" )
echo "got: $result"

echo "── pipeline launches subshells ──"
v=parent
echo data | { read l; v=changed; echo "in subshell: v=$v"; }
echo "parent: v=$v (depends on pipefail and shell flags)"

# === ztest assertions ===
# subshell var-leak: ( x=inner ) MUST NOT change outer x
xx=outer
( xx=inner )
zassert_eq "$xx" "outer"  "subshell does not leak assignment"
# group {} MUST change outer y
yy=outer
{ yy=inner; }
zassert_eq "$yy" "inner"  "group preserves scope"
# nested subshells: outer z unchanged
zz=outer
( zz=middle; ( zz=innermost ) )
zassert_eq "$zz" "outer"  "nested subshell isolation"
# exit code of subshell
( exit 7 )
zassert_eq "$?" "7"   "subshell exit code propagates"
# command sub captures stdout
zassert_eq "$( echo hello )" "hello"  "command sub capture"
# pipeline rightmost wins (no pipefail)
( exit 7 ) | ( exit 11 )
zassert_eq "$?" "11"  "pipe rightmost exit"
ztest_run
