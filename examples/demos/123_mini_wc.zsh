#!/usr/bin/env zshrs
# Mini-wc in pure zsh — line/word/char counts.

mini_wc() {
    local opt count_lines=0 count_words=0 count_chars=0
    while getopts "lwc" opt; do
        case $opt in
            l) count_lines=1 ;;
            w) count_words=1 ;;
            c) count_chars=1 ;;
        esac
    done
    shift $(( OPTIND - 1 ))
    OPTIND=1
    if (( ! count_lines && ! count_words && ! count_chars )); then
        count_lines=1
        count_words=1
        count_chars=1
    fi

    local file lines words chars total_lines=0 total_words=0 total_chars=0 file_count=0
    for file in "${@:-/dev/stdin}"; do
        lines=0
        words=0
        chars=0
        if [[ $file == /dev/stdin ]]; then
            while IFS= read -r line; do
                (( lines++ ))
                local -a wa=($=line)
                (( words += ${#wa[@]} ))
                (( chars += ${#line} + 1 ))  # +1 for newline
            done
        else
            while IFS= read -r line; do
                (( lines++ ))
                local -a wa=($=line)
                (( words += ${#wa[@]} ))
                (( chars += ${#line} + 1 ))
            done < "$file"
        fi
        local out=""
        (( count_lines )) && out+=$(printf "%8d" $lines)
        (( count_words )) && out+=$(printf "%8d" $words)
        (( count_chars )) && out+=$(printf "%8d" $chars)
        [[ $file != /dev/stdin ]] && out+=" $file"
        echo "$out"
        (( total_lines += lines ))
        (( total_words += words ))
        (( total_chars += chars ))
        (( file_count++ ))
    done
    if (( file_count > 1 )); then
        local out=""
        (( count_lines )) && out+=$(printf "%8d" $total_lines)
        (( count_words )) && out+=$(printf "%8d" $total_words)
        (( count_chars )) && out+=$(printf "%8d" $total_chars)
        out+=" total"
        echo "$out"
    fi
}

tmpdir=/tmp/zshrs_miniwc_$$
mkdir -p "$tmpdir"
trap "rm -rf $tmpdir" EXIT

cat > "$tmpdir/a.txt" <<EOF
one two three
four five
six
EOF

cat > "$tmpdir/b.txt" <<EOF
alpha beta
gamma delta epsilon
EOF

echo "── default all ──"
mini_wc "$tmpdir/a.txt"

echo "── -l lines only ──"
mini_wc -l "$tmpdir/a.txt"

echo "── -w words only ──"
mini_wc -w "$tmpdir/a.txt"

echo "── -c chars only ──"
mini_wc -c "$tmpdir/a.txt"

echo "── multiple files ──"
mini_wc "$tmpdir/a.txt" "$tmpdir/b.txt"

echo "── from stdin ──"
printf 'aaa bbb\nccc\n' | mini_wc

# === ztest assertions ===
td=/tmp/zshrs_miniwc_test_$$
mkdir -p "$td"
printf 'one two three\nfour five\nsix\n' > "$td/a.txt"
# -l: 3 lines
zassert_eq "$(mini_wc -l $td/a.txt | awk '{print $1}')"  "3"   "mini_wc -l = 3"
# -w: 6 words (3+2+1)
zassert_eq "$(mini_wc -w $td/a.txt | awk '{print $1}')"  "6"   "mini_wc -w = 6"
# -c: 28 chars (13+1+9+1+3+1) = 28
zassert_eq "$(mini_wc -c $td/a.txt | awk '{print $1}')"  "28"  "mini_wc -c = 28"
# Default all-three on one line
default_line=$(mini_wc $td/a.txt | awk '{print $1, $2, $3}')
zassert_eq "$default_line"  "3 6 28"  "mini_wc default columns"
# stdin path
zassert_eq "$(printf 'aaa bbb\nccc\n' | mini_wc -l | awk '{print $1}')"  "2"  "mini_wc -l stdin"
rm -rf "$td"
ztest_run
