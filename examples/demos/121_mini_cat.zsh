#!/usr/bin/env zshrs
# Mini-cat in pure zsh — implement cat as a function (no fork).
# Demonstrates Src/builtin.c bin_read pattern + heredoc reading.

mini_cat() {
    local opt
    local -i show_lineno=0
    local -i show_ends=0
    local -i squeeze_blank=0
    # Parse short options like real cat.
    while getopts "nEs" opt; do
        case $opt in
            n) show_lineno=1 ;;
            E) show_ends=1 ;;
            s) squeeze_blank=1 ;;
        esac
    done
    shift $(( OPTIND - 1 ))
    OPTIND=1

    local file lineno=0 prev_blank=0
    for file in "$@"; do
        if [[ ! -f $file ]]; then
            echo "mini_cat: $file: no such file" >&2
            continue
        fi
        while IFS= read -r line || [[ -n $line ]]; do
            if (( squeeze_blank )) && [[ -z $line ]]; then
                if (( prev_blank )); then continue; fi
                prev_blank=1
            else
                prev_blank=0
            fi
            (( lineno++ ))
            if (( show_lineno )); then
                printf "%6d  %s" $lineno "$line"
            else
                printf "%s" "$line"
            fi
            if (( show_ends )); then
                printf "\$"
            fi
            printf "\n"
        done < "$file"
    done
}

tmpdir=/tmp/zshrs_minicat_$$
mkdir -p "$tmpdir"
trap "rm -rf $tmpdir" EXIT

cat > "$tmpdir/a.txt" <<EOF
alpha
beta

gamma


delta
EOF

cat > "$tmpdir/b.txt" <<EOF
file b first
file b second
EOF

echo "── plain ──"
mini_cat "$tmpdir/a.txt"

echo "── -n line numbers ──"
mini_cat -n "$tmpdir/a.txt"

echo "── -E show ends ──"
mini_cat -E "$tmpdir/a.txt"

echo "── -s squeeze blanks ──"
mini_cat -s "$tmpdir/a.txt"

echo "── -ns combined ──"
mini_cat -ns "$tmpdir/a.txt"

echo "── multiple files ──"
mini_cat -n "$tmpdir/a.txt" "$tmpdir/b.txt"

echo "── missing file ──"
mini_cat -n /nonexistent_xyz 2>&1
