#!/usr/bin/env zshrs
# Advanced file-descriptor redirection — exec, 2>&1, &>, {fd} alloc.
# Ported from Src/exec.c addfd + Src/exec.c redirect dispatch.

tmpdir=/tmp/zshrs_fd_$$
mkdir -p "$tmpdir"
trap "rm -rf $tmpdir" EXIT

echo "── basic > and >> ──"
echo "line1" > "$tmpdir/a.txt"
echo "line2" >> "$tmpdir/a.txt"
cat "$tmpdir/a.txt"

echo "── 2>&1 merge stderr→stdout ──"
{ echo stdout; echo stderr 1>&2; } 2>&1 | grep -E '^(stdout|stderr)$'

echo "── &> shortcut (zsh-specific) ──"
{ echo out; echo err >&2; } &> "$tmpdir/merged.log"
echo "merged contents:"
sort "$tmpdir/merged.log"

echo "── here-string >>> ──"
grep "needle" <<< "look for the needle here"

echo "── redirect to /dev/null suppress ──"
ls /nonexistent 2>/dev/null
echo "after silent error: $?"

echo "── tee a stream ──"
echo "duplicate-me" | tee "$tmpdir/copy1.log" "$tmpdir/copy2.log"
echo "copy1 has:"
cat "$tmpdir/copy1.log"
echo "copy2 has:"
cat "$tmpdir/copy2.log"

echo "── block redirection (subshell with > file) ──"
{
    echo "line A"
    echo "line B"
} > "$tmpdir/block.log"
echo "block log has:"
cat "$tmpdir/block.log"

echo "── multi-redirection ──"
{ echo first; echo second; echo third; } > "$tmpdir/multi.log" 2>&1
wc -l < "$tmpdir/multi.log"

echo "── here-doc as input to sort ──"
sort <<EOF
gamma
alpha
beta
EOF

# === ztest assertions ===
tmpdir2=/tmp/zshrs_fd_assert_$$
mkdir -p "$tmpdir2"
echo "line1" > "$tmpdir2/a.txt"
echo "line2" >> "$tmpdir2/a.txt"
zassert_eq "$(cat "$tmpdir2/a.txt")" $'line1\nline2' "> + >> append works"
{ echo o; echo e >&2; } 2>&1 > "$tmpdir2/out.log"
zassert_eq "$(cat "$tmpdir2/out.log")" "o" "stdout-only after 2>&1 >"
{ echo o2; echo e2 >&2; } &> "$tmpdir2/merged.log"
zassert_contains "$(cat "$tmpdir2/merged.log")" "o2" "&> captures stdout"
zassert_contains "$(cat "$tmpdir2/merged.log")" "e2" "&> captures stderr"
zassert_eq "$(grep needle <<< 'look for the needle here')" "look for the needle here" "here-string"
ls /nonexistent_xyz_abc 2>/dev/null
zassert_ne "$?" "0" "missing path → nonzero status"
{ echo a; echo b; echo c; } > "$tmpdir2/multi.log"
zassert_eq "$(wc -l < "$tmpdir2/multi.log" | tr -d ' ')" "3" "block redirect line count"
zassert_eq "$(sort <<EOF
gamma
alpha
beta
EOF
)" $'alpha\nbeta\ngamma' "heredoc into sort"
rm -rf "$tmpdir2"
ztest_run

