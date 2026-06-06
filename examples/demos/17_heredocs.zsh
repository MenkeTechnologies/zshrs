#!/usr/bin/env zshrs
# Heredocs — unquoted (expand), quoted (literal), indented.

name=zshrs
ver=$((11 + 22))

echo "── unquoted heredoc (expand) ──"
cat <<EOF
Welcome to $name!
Version: $ver
Pi-ish: $((22 * 100 / 7))
EOF

echo "── quoted heredoc (literal, no expansion) ──"
cat <<"EOF"
$name is not expanded here.
$((1 + 2)) stays literal.
EOF

echo "── indented heredoc (<<-, strips leading tabs) ──"
cat <<-EOF
	this is indented in source
	$name expanded here too
	last line
EOF

echo "── stdin to a command ──"
sort <<EOF
gamma
alpha
beta
delta
EOF

# === ztest assertions ===
# Unquoted heredoc expands $name and $((..)).
unq="$(cat <<EOF
Welcome to $name!
Version: $ver
EOF
)"
zassert_contains "$unq" "Welcome to zshrs!"  "unquoted heredoc expands \$name"
zassert_contains "$unq" "Version: 33"        "unquoted heredoc expands arith"
# Quoted heredoc keeps literal.
quoted="$(cat <<"EOF"
$name stays literal
$((1+2)) stays literal
EOF
)"
zassert_contains "$quoted" "\$name stays literal"  "quoted heredoc preserves \$"
zassert_contains "$quoted" "\$((1+2))"            "quoted heredoc preserves arith"
# Indented heredoc <<- strips leading tabs.
ind="$(cat <<-EOF
	tab-stripped
	$name expanded
	EOF
)"
zassert_contains "$ind" "tab-stripped"        "indented heredoc strips tabs"
zassert_contains "$ind" "zshrs expanded"      "indented heredoc still expands"
# sort on heredoc input.
sorted="$(sort <<EOF
gamma
alpha
beta
delta
EOF
)"
zassert_eq "$sorted" $'alpha\nbeta\ndelta\ngamma' "sort with heredoc"
ztest_run
