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

# === ztest assertions ===
td=/tmp/zshrs_minicat_test_$$
mkdir -p "$td"
printf 'alpha\nbeta\ngamma\n' > "$td/x.txt"
# Plain pass-through: 3 lines.
zassert_eq "$(mini_cat $td/x.txt)" "alpha
beta
gamma"  "mini_cat plain"
# -n prepends numbered prefix
out=$(mini_cat -n $td/x.txt)
zassert_contains "$out" "1  alpha"  "mini_cat -n line 1"
zassert_contains "$out" "3  gamma"  "mini_cat -n line 3"
# -E appends $ at line end
out_e=$(mini_cat -E $td/x.txt)
zassert_contains "$out_e" "alpha$"  "mini_cat -E end marker"
# Missing file → stderr message; stdout empty
err=$(mini_cat /nonexistent_xyz_test 2>&1 >/dev/null)
zassert_contains "$err" "no such file"  "mini_cat missing"
# Squeeze blanks: input with consecutive empty lines → single blank between blocks
printf 'one\n\n\n\ntwo\n' > "$td/b.txt"
squeezed=$(mini_cat -s $td/b.txt)
# 'one' + 1 blank + 'two' = 3 lines
zassert_eq "$(echo "$squeezed" | wc -l | tr -d ' ')" "3"  "mini_cat -s squeezes blanks"
rm -rf "$td"
ztest_run
