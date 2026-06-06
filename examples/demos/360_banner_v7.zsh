#!/usr/bin/env zshrs
# Grand finale v7 — 360 demos.

banner() {
    local txt=$1 width=${2:-72}
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

  ██████  ██████  ████████  ████████  █████  ██████   ██████
 ██      ██    ██ ██        ██       ██   ██ ██   ██ ██
  ██████ ██    ██ ████████  ██       ███████ ██████   ██████
       ██████  ██ ██     ██ ██       ██   ██ ██    ██      ██
  ██████        ██ ████████  ████████ █████  ██   ███ ██████


EOF

banner "🎯 zshrs DEMO HARNESS: 360 STRONG 🎯" 72
echo
echo "  zshrs is the first compiled Unix shell — a Rust port of zsh"
echo "  with bytecode + fusevm JIT + AOP intercepts + worker pool."
echo "  This 360-demo suite pins the compat surface against zsh's"
echo "  Src/*.c port-site by port-site. Every demo runs on CI."
echo

banner "BATCH 14 (336-360) THEMES" 72
batches=(
    "336|Roman numeral encode/decode + validate"
    "337|trie (insert/search/autocomplete/count_prefix)"
    "338|Z-function + Z-based substring search"
    "339|longest common substring + brute verify"
    "340|longest palindromic subsequence DP"
    "341|Nim game + XOR theorem + optimal play"
    "342|peg solitaire + greedy solver"
    "343|RFC 2822 date parser + epoch conversion"
    "344|ISO 8601 parser (date/time/duration/week)"
    "345|Pollard's rho factorization"
    "346|continued fractions + sqrt approx + φ"
    "347|columnar transposition + rail fence ciphers"
    "348|color conversions (RGB/HSL/HSV/256/hex)"
    "349|\$ZSH_EVAL_CONTEXT + \$FUNCNEST + \$ZSH_SUBSHELL"
    "350|🎉 demo-350 milestone banner"
    "351|Sokoban small board + scripted moves"
    "352|text wrap (greedy/justify/center/right)"
    "353|Unicode utilities (width/escape/byte count)"
    "354|URL encode/decode (RFC 3986) + parser"
    "355|calendar (month grid + year overview + Zeller)"
    "356|disjoint-set union (Kruskal + grid islands)"
    "357|priority queue (heap + Dijkstra)"
    "358|funcstack/funcfiletrace/functrace/funcsourcetrace"
    "359|comprehensive parameter expansion flags"
    "360|grand finale 360-demo banner v7"
)
for b in "${batches[@]}"; do
    printf "  %s  %s\n" "${b%%|*}" "${b#*|}"
done

echo
banner "CUMULATIVE BREAKDOWN" 72
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
    "336-360|history math + parsers + ciphers + zsh metadata"
)
printf "  %-10s | %s\n" "Range" "Theme"
printf "  %-10s + %s\n" "──────────" "─────"
for b in "${prev[@]}"; do
    printf "  %-10s | %s\n" "${b%%|*}" "${b#*|}"
done

echo
banner "META" 72
echo
echo "  demos:        360"
echo "  pid:          $$"
echo "  zsh version:  $ZSH_VERSION"
zmodload zsh/datetime 2>/dev/null
if (( ${+EPOCHSECONDS} )); then
    echo "  generated:    $(TZ=UTC strftime '%Y-%m-%d %H:%M UTC' $EPOCHSECONDS 2>/dev/null)"
fi
echo "  CI runtime:   ~50s parallel (361 tests + coverage pin)"
echo

banner "→ github.com/MenkeTechnologies/zshrs" 72

# === ztest assertions ===
out=$(banner "hi" 10)
zassert_contains "$out" "═════════"           "banner has horizontal rule"
zassert_contains "$out" "hi"                  "banner contains text"
zassert_eq "$(echo "$out" | wc -l | tr -d ' ')" "3" "banner is 3 lines"
zassert_eq "${batches[1]%%|*}" "336"          "batches[1] index 336"
zassert_eq "${prev[1]%%|*}" "001-030"         "prev[1] range start"
zassert_eq "${#batches}" "25"                 "25 batch entries (336-360)"
ztest_run
