echo "$'ANSI \x1b[31mred\x1b[0m'"
echo $'tab\there'
echo $'\nnewline'
echo $'\\backslash'
str=$'quoted with \'apostrophe\''
mixed="prefix$'\t'suffix"
escaped=\"literal\"
multiline="$(cat <<EOF
line 1
line 2
EOF
)"
