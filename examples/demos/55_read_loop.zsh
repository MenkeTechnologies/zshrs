#!/usr/bin/env zshrs
# Line-by-line read pattern — process each line from stdin / heredoc.

echo "── plain read ──"
count=0
while read -r line; do
    (( count++ ))
    printf "[%2d] %s\n" $count "$line"
done <<EOF
alpha
beta
gamma
delta
epsilon
EOF
echo "total: $count lines"

echo "── word per line ──"
typeset -A wordcount
while read -r line; do
    for w in $=line; do
        (( wordcount[$w]++ ))
    done
done <<EOF
the quick brown fox
the lazy dog
the brown fox sleeps
EOF
for k v in "${(@kv)wordcount}"; do
    printf "%-10s %d\n" "$k" "$v"
done | sort

echo "── tab-separated columns ──"
while IFS=$'\t' read -r col1 col2 col3; do
    echo "[$col1] [$col2] [$col3]"
done <<EOF
alice	30	admin
bob	25	user
carol	35	guest
EOF

echo "── numbered like nl ──"
n=0
while read -r line; do
    (( n++ ))
    printf "%4d\t%s\n" $n "$line"
done <<EOF
first
second
third
fourth
EOF

# === ztest assertions ===
zassert_eq "$count"             5  "plain read counted 5 lines"
zassert_eq "${wordcount[the]}"  3  "'the' counted 3x via read+split"
zassert_eq "${wordcount[fox]}"  2  "'fox' counted 2x"
zassert_eq "${wordcount[brown]}" 2 "'brown' counted 2x"
zassert_eq "${wordcount[quick]}" 1 "'quick' counted once"
zassert_eq "${#wordcount[@]}"   7  "7 unique words across heredoc"
zassert_eq "$n"                 4  "nl-style numbered 4 lines"
ztest_run
