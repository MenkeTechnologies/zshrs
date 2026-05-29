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
