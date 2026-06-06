#!/usr/bin/env zshrs
# Grand finale v6 — 335 demos.

banner() {
    local txt=$1 width=${2:-70}
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

  ███████╗███████╗██╗  ██╗██████╗ ███████╗
  ╚══███╔╝██╔════╝██║  ██║██╔══██╗██╔════╝
    ███╔╝ ███████╗███████║██████╔╝███████╗
   ███╔╝  ╚════██║██╔══██║██╔══██╗╚════██║
  ███████╗███████║██║  ██║██║  ██║███████║
  ╚══════╝╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝

EOF

banner "🌌 zshrs DEMO HARNESS: 335 STRONG 🌌" 70
echo
echo "  zshrs — the first compiled Unix shell. Bytecode + fusevm JIT"
echo "  + AOP + worker pool. Drop-in zsh replacement targeting the"
echo "  zsh-130k-LOC compat floor as a ceiling, not a baseline."
echo

banner "BATCH 13 (311-335) THEMES" 70
batches=(
    "311|binary search tree (insert/search/inorder/height)"
    "312|AVL tree w/ rotations + balance factor"
    "313|Bloom filter v2 + union/intersect"
    "314|deque (monotonic for sliding-window max)"
    "315|ring buffer (fixed-size FIFO w/ overwrite)"
    "316|IPv4 subnet calculator (CIDR/mask/broadcast/contains)"
    "317|MAC address parser + OUI vendor lookup"
    "318|file checksums (adler32/djb2/fnv1a/sum16/xor)"
    "319|anagram solver (canonical sort + grouping)"
    "320|leet speak basic + advanced + random"
    "321|pig latin encode + reverse"
    "322|zsh HISTFILE parser (extended + raw)"
    "323|SSH known_hosts parser (plain + hashed)"
    "324|brace expansion (numeric/alpha/nested/product)"
    "325|print -r/-R/-D/-aC/-P/-v/-u/-s/-z/-N/-m flags"
    "326|compinit + completion lifecycle"
    "327|extended_glob deep dive (^/~/#/##/<>/(#i)/(#a))"
    "328|assoc array (kv) flag + 2-dim + invert + merge"
    "329|max subarray (Kadane + circular + stock profit)"
    "330|LIS + LCS + edit distance DP"
    "331|0/1 knapsack + fractional comparison"
    "332|coin change (min + count ways + reconstruction)"
    "333|topological sort (Kahn's + cycle detect)"
    "334|LRU cache (doubly-linked list + hash)"
    "335|grand finale 335-demo banner v6"
)
for b in "${batches[@]}"; do
    printf "  %s  %s\n" "${b%%|*}" "${b#*|}"
done

echo
banner "CUMULATIVE BREAKDOWN" 70
prev=(
    "001-030|shell fundamentals"
    "031-060|algorithms + data structures"
    "061-085|zsh C feature pins"
    "086-110|advanced runtime"
    "111-135|extension + utility"
    "136-160|systems + apps"
    "161-185|utilities + meta"
    "186-210|parsers + apps"
    "211-235|apps + games + introspection"
    "236-260|hooks + cryptography + grids + parsers"
    "261-285|crypto + graphs + games + zsh hooks"
    "286-310|trees + games + strings + zsh internals"
    "311-335|trees + DP + zsh deep dives"
)
printf "  %-10s | %s\n" "Range" "Theme"
printf "  %-10s + %s\n" "──────────" "─────"
for b in "${prev[@]}"; do
    printf "  %-10s | %s\n" "${b%%|*}" "${b#*|}"
done

echo
banner "PORT BUGS DOCUMENTED" 70
echo
echo "  docs/BUGS.md catalogs 15 issues found while writing demos:"
echo
echo "  Demo-errors (zsh agrees, demo was wrong): #1, #2, #3, #6"
echo "  Fixed port bugs: #5 (prompt escapes), #7 (local arr=( \$=s ))"
echo "  Open port-bugs:"
echo "    #4  anon-fn in cmd-sub silent abort"
echo "    #8  local IFS= leaks past return"
echo "    #9  \${arr[(expr)*N+M]} unquoted → empty"
echo "    #10 nested-for + cmd-sub in fn corrupts iteration"
echo "    #11 printf \"%d\" \"' \" returns 0 (space char)"
echo "    #12 \${var%|*} treats | as glob alt"
echo "    #13 [[ \"\$x\" == \"?\" ]] ignores quotes"
echo "    #14 [[ \$ch == \"{\" ]] parse error"
echo "    #15 set -- \${=x} mis-iter in fn"
echo

banner "META" 70
echo
echo "  demos:        335"
echo "  pid:          $$"
echo "  zsh version:  $ZSH_VERSION"
zmodload zsh/datetime 2>/dev/null
if (( ${+EPOCHSECONDS} )); then
    echo "  generated:    $(TZ=UTC strftime '%Y-%m-%d %H:%M UTC' $EPOCHSECONDS 2>/dev/null)"
fi
echo "  CI runtime:   ~35s parallel (336 tests + coverage pin)"
echo

banner "→ github.com/MenkeTechnologies/zshrs" 70

# === ztest assertions ===
zassert_eq "${#batches}" 25 "25 entries in batch 13"
zassert_eq "${#prev}"    13 "13 cumulative ranges"
zassert_contains "${batches[1]}" "binary search tree" "311 entry"
zassert_contains "${batches[25]}" "grand finale"     "335 entry"
zassert_contains "$(banner hello 20)" "hello"        "banner emits text"
zassert_ok "$ZSH_VERSION" "ZSH_VERSION nonempty"
zassert_ok "$$"           "pid available"
ztest_run
