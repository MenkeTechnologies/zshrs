#!/usr/bin/env zshrs
# Milestone — 200 demos! Banner + features so far.

print_banner() {
    local txt=$1 width=${2:-50}
    local n=${#txt}
    local pad=$(( (width - n) / 2 ))
    local sp=""; local i
    for ((i=0; i<pad; i++)); do sp+=" "; done
    local bar=""
    for ((i=0; i<width; i++)); do bar+="="; done
    echo "$bar"
    echo "${sp}${txt}"
    echo "$bar"
}

print_banner "ZSHRS DEMO MILESTONE — 200" 50
echo
echo "An aggregate of pure-shell demos pinning every zshrs"
echo "feature against its upstream zsh C source."
echo

echo "── Stats ──"
echo "  Total demos:    200"
echo "  CI runtime:     ~30s"
echo "  Run via:        cargo test --test examples_demos_ci"
echo

echo "── Coverage by category ──"
categories=(
    "Shell fundamentals:    30"
    "Algorithms+structures: 30"
    "Zsh C features:        25"
    "Advanced runtime:      25"
    "Extensions+utility:    25"
    "Systems+apps:          25"
    "Utilities+meta:        25"
    "Specialty patterns:    15"
)
for c in "${categories[@]}"; do
    echo "  $c"
done

echo
print_banner "200 STRONG" 50

# Show a few inline mini-demos.
echo
echo "── inline mini-demos ──"

echo "1. arithmetic: $(( 2 ** 16 ))"
echo "2. param flag: ${(j: + :)$(echo 1 2 3)}"
echo "3. assoc:      $(typeset -A x=(k v); echo ${x[k]})"
echo "4. regex:      $([[ abc123 =~ "([a-z]+)([0-9]+)" ]] && echo "letters=${match[1]} digits=${match[2]}")"
echo "5. backtick:   $(date +%H:%M 2>/dev/null || echo --:--)"

echo
echo "── thank you ──"
echo "  ───────────────────────────"
echo "  zshrs: 200 demos strong"
echo "  every one runs on CI"
echo "  every one cites Src/*.c"
echo "  ───────────────────────────"

# === ztest assertions ===
zassert_eq "${#categories[@]}" 8                "8 categories listed"
zassert_contains "$(print_banner 'X' 10)" "=========="   "banner draws ="
zassert_contains "$(print_banner 'X' 10)" "    X"        "banner centers text"
zassert_eq $((2 ** 16))  65536                   "2^16 mini-demo"
zassert_eq "$(typeset -A x=(k v); echo ${x[k]})" "v"     "assoc inline mini-demo"
[[ abc123 =~ ([a-z]+)([0-9]+) ]] && zassert_eq "$match[1]" "abc" "regex match[1]"
ztest_run
