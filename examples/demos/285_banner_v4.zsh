#!/usr/bin/env zshrs
# Grand finale v4 — 285 demos.

banner() {
    local txt=$1 width=${2:-66}
    local n=${#txt}
    local pad=$(( (width - n) / 2 ))
    local sp="" bar="" i
    for ((i=0; i<pad; i++)); do sp+=" "; done
    for ((i=0; i<width; i++)); do bar+="═"; done
    echo "$bar"
    echo "${sp}${txt}"
    echo "$bar"
}

cat <<'EOF'

     ██████  ███████ ██   ██ ██████  ███████
        ███  ██      ██   ██ ██   ██ ██
       ███   ███████ ███████ ██████  ███████
      ███         ██ ██   ██ ██   ██      ██
     ██████  ███████ ██   ██ ██   ██ ███████

EOF

banner "🎉 ZSHRS 285-DEMO PARITY HARNESS 🎉" 66
echo
echo "  Compiled Unix shell. Drop-in zsh replacement."
echo "  Every demo runs on CI under zshrs --zsh."
echo "  Every demo cites the Src/*.c port it exercises."
echo

banner "BATCH 11 (261–285) THEMES" 66
batches=(
    "261|prime factorization (trial div + canonical form)"
    "262|Miller-Rabin probabilistic primality"
    "263|extended Euclidean + modular inverse + RSA-toy"
    "264|A* pathfinding on ASCII grid"
    "265|Kruskal's MST (union-find + path compression)"
    "266|Prim's MST (grow from start)"
    "267|Floyd-Warshall all-pairs shortest paths"
    "268|Bellman-Ford + negative-cycle detect"
    "269|N-Queens (count + render)"
    "270|15-puzzle slide + inversion-count solvability"
    "271|Towers of Hanoi w/ animated render"
    "272|Markdown→HTML converter"
    "273|HTTP request parser (method/path/headers/body)"
    "274|log format auto-detector"
    "275|CSV inner+outer join + aggregation"
    "276|Blackjack w/ dealer-17 rule"
    "277|dice probability + Yahtzee patterns"
    "278|Rock-Paper-Scissors round-robin tournament"
    "279|XOR cipher + frequency analysis"
    "280|One-Time Pad w/ key-reuse weakness demo"
    "281|periodic/precmd/preexec/chpwd hooks"
    "282|positional params + getopts deep dive"
    "283|trap matrix (EXIT/ERR/ZERR/USR1/INT/TERM/HUP)"
    "284|atomic file write via tmp+rename"
    "285|grand finale v4 banner"
)
for b in "${batches[@]}"; do
    n="${b%%|*}"
    desc="${b#*|}"
    printf "  %s  %s\n" "$n" "$desc"
done

echo
banner "CUMULATIVE BATCH SUMMARY" 66
prev=(
    "01-30|fundamentals"
    "31-60|algorithms + data structures"
    "61-85|zsh C feature pins"
    "86-110|advanced runtime"
    "111-135|extension + utility"
    "136-160|systems + apps"
    "161-185|utilities + meta"
    "186-210|parsers + apps + meta"
    "211-235|meta + games + apps"
    "236-260|hooks + cryptography + grids + parsers"
    "261-285|crypto + graphs + games + zsh hooks"
)
printf "  %-10s | %s\n" "Range" "Theme"
printf "  %-10s + %s\n" "──────────" "─────"
for b in "${prev[@]}"; do
    printf "  %-10s | %s\n" "${b%%|*}" "${b#*|}"
done

echo
banner "META" 66
echo
echo "  demos:        285"
echo "  pid:          $$"
echo "  zsh version:  $ZSH_VERSION"
echo "  argv0:        $0"

zmodload zsh/datetime 2>/dev/null
if (( ${+EPOCHSECONDS} )); then
    echo "  generated:    $(TZ=UTC strftime '%Y-%m-%d %H:%M UTC' $EPOCHSECONDS 2>/dev/null)"
fi

echo
banner "ZSHRS — THE FIRST COMPILED UNIX SHELL" 66
echo
echo "  Bytecode + fusevm + AOP + worker pool."
echo "  No fork, no problems."
echo "  → github.com/MenkeTechnologies/zshrs"
echo

banner "" 66

# === ztest assertions ===
# banner produces 3 lines for arbitrary text (top bar, padded text, bottom bar)
banner_lines=$(banner "hi" 20 | wc -l)
# wc -l on macOS pads with spaces — strip via arith context
zassert_eq "$(( banner_lines + 0 ))" 3 "banner emits 3 lines"
# batch index has exactly 25 entries (demos 261-285)
zassert_eq "${#batches}" 25 "batches array has 25 entries (261-285)"
# prev summary covers 11 prior batches
zassert_eq "${#prev}" 11 "11 cumulative batch summaries"
# First batch is 261 prime factorization
zassert_match '^261\|' "${batches[1]}" "first batch entry is 261"
# Last batch is 285
zassert_match '^285\|' "${batches[25]}" "last batch entry is 285"
# ZSH_VERSION populated
zassert_ne "$ZSH_VERSION" "" "ZSH_VERSION is set"
# argv0 should be 'banner' (last fn call seen) or the script — runtime-dependent
zassert_ne "$0" "" "\$0 not empty"
zassert_eq "$$" "$(echo $$)" "\$\$ stable"
ztest_run
