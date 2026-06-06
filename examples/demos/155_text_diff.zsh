#!/usr/bin/env zshrs
# Naive line-level diff between two arrays.

diff_lines() {
    local -a a=("${(@f)1}")
    local -a b=("${(@f)2}")
    local -A in_a in_b
    for x in "${a[@]}"; do in_a[$x]=1; done
    for x in "${b[@]}"; do in_b[$x]=1; done

    echo "── only in A ──"
    for x in "${a[@]}"; do
        [[ -z ${in_b[$x]+x} ]] && echo "  - $x"
    done

    echo "── only in B ──"
    for x in "${b[@]}"; do
        [[ -z ${in_a[$x]+x} ]] && echo "  + $x"
    done

    echo "── common (preserving A's order) ──"
    for x in "${a[@]}"; do
        [[ -n ${in_b[$x]+x} ]] && echo "    $x"
    done
}

a=$'alpha\nbeta\ngamma\ndelta'
b=$'beta\ngamma\nepsilon\nzeta'

diff_lines "$a" "$b"

echo "── another pair ──"
old=$'one\ntwo\nthree\nfour'
new=$'one\nTWO\nthree\nfive'
diff_lines "$old" "$new"

echo "── word-level diff (sentence A vs B) ──"
word_diff() {
    local s1=$1 s2=$2
    local -A in1 in2
    local w
    for w in $=s1; do in1[$w]=1; done
    for w in $=s2; do in2[$w]=1; done
    echo "removed: "
    for w in $=s1; do
        [[ -z ${in2[$w]+x} ]] && printf "  -%s" "$w"
    done
    echo
    echo "added:   "
    for w in $=s2; do
        [[ -z ${in1[$w]+x} ]] && printf "  +%s" "$w"
    done
    echo
}

word_diff \
    "the quick brown fox jumps over the lazy dog" \
    "the slow brown fox jumps over the active dog"

# === ztest assertions ===
# Re-run the diff and capture into vars for assertion.
a=$'alpha\nbeta\ngamma\ndelta'
b=$'beta\ngamma\nepsilon\nzeta'
out="$(diff_lines "$a" "$b")"
zassert_contains "$out" "- alpha"       "alpha only in A"
zassert_contains "$out" "- delta"       "delta only in A"
zassert_contains "$out" "+ epsilon"     "epsilon only in B"
zassert_contains "$out" "+ zeta"        "zeta only in B"
zassert_contains "$out" "beta"          "beta in common"
zassert_contains "$out" "gamma"         "gamma in common"
# word_diff smoke
wout="$(word_diff "the quick brown fox" "the slow brown fox")"
zassert_contains "$wout" "quick"        "word_diff removed quick"
zassert_contains "$wout" "slow"         "word_diff added slow"
ztest_run
