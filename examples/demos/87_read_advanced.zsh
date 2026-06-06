#!/usr/bin/env zshrs
# `read` builtin advanced — -r raw, -d delim, -A array, -k count, -l line.
# Ported from Src/builtin.c bin_read.

echo "── basic read line ──"
echo "hello world" | { read -r line; echo "got: $line"; }

echo "── read -d (custom delimiter) ──"
printf 'alpha|beta|gamma|' | { read -d '|' a; read -d '|' b; read -d '|' c
    echo "a=$a b=$b c=$c"
}

echo "── read -A (array — words into array) ──"
echo "one two three four five" | { read -A arr
    echo "n=${#arr[@]}"
    for x in "${arr[@]}"; do echo "  - $x"; done
}

echo "── multiple vars from one line ──"
echo "name age role" | { read -r name age role
    echo "n=$name a=$age r=$role"
}

echo "── read with IFS override ──"
printf 'one:two:three:four\n' | { IFS=: read -r a b c d
    echo "a=$a b=$b c=$c d=$d"
}

echo "── read with leftover capture ──"
echo "a b c d e f g" | { read -r first second rest
    echo "first=$first second=$second rest=$rest"
}

echo "── read all lines into array ──"
printf 'line1\nline2\nline3\n' | {
    lines=()
    while read -r l; do lines+=("$l"); done
    echo "count=${#lines[@]}"
    for ((i=1; i<=${#lines[@]}; i++)); do
        printf "[%d] %s\n" $i "${lines[i]}"
    done
}

# === ztest assertions ===
# Use $() capture because read happens inside the subshell.
zassert_eq "$(echo 'hello world' | { read -r line; echo "$line"; })" "hello world" "read -r basic"
zassert_eq "$(echo 'one two three' | { read -A a; echo "${#a[@]} ${a[2]}"; })" "3 two" "read -A array"
zassert_eq "$(echo 'name age role' | { read -r n a r; echo "$n|$a|$r"; })" "name|age|role" "multi-var read"
zassert_eq "$(printf 'one:two:three:four\n' | { IFS=: read -r a b c d; echo "$a|$b|$c|$d"; })" "one|two|three|four" "IFS=: split"
zassert_eq "$(echo 'a b c d e f g' | { read -r f s rest; echo "$f|$s|$rest"; })" "a|b|c d e f g" "leftover capture"
nlines=$(printf 'l1\nl2\nl3\n' | { c=0; while read -r x; do (( c++ )); done; echo $c; })
zassert_eq "$nlines" "3" "while read line count"
zassert_eq "$(printf 'alpha|beta|gamma|' | { read -d '|' x; echo "$x"; })" "alpha" "read -d delim"
ztest_run

