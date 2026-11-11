echo "a b" | string replace -r " " "\\t" | od -c | head -1
echo "a b" | string replace -r "(a) (b)" "\$2\\n\$1"
echo abc | string replace -r "(b)" "\\U\$1\\E!"
echo abc | string replace -r "b" "\\\\" 2>/dev/null; echo $status
echo hello | string replace -r "(h)(e)" "\\u\$1\$2"
echo abc | string replace -r b "\$\$"
echo abc | string replace -r b "\\x41"
