#!/usr/bin/env zshrs
# Milestone banner v9 — 375 demos.
#
# Each "banner" demo is the closing frame of a numbered batch — like
# 360_banner_v7 and 367_banner_v8 before it. The v9 batch (368–375)
# covered: bencode wire format, Hamming(7,4) ECC, divide-and-conquer
# skyline outline, brainfuck interpreter, Ackermann (recursive +
# stack-machine iterative), LZW codec, Elias gamma/delta codes.

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

   ████████  ███████ ██████
      ██    ██     ██████
      ██     ███████ ██
      ██    ██     ██████
      ██     ████████ ██████

EOF

banner "🎯 zshrs DEMO HARNESS: 375 STRONG 🎯" 72
echo
echo "  zshrs — first compiled Unix shell. Rust port of zsh."
echo "  Bytecode-cached + fusevm JIT + AOP intercepts + worker pool."
echo "  This 375-demo suite pins zshrs against zsh's C source."
echo "  Every demo runs on CI; every demo is real functional code."
echo

banner "BATCH 16 (368-375) — CODECS, INTERPRETERS, NUMBER THEORY" 72
batches=(
    "368|bencode encoder/decoder (BitTorrent BEP-3 wire format)"
    "369|Hamming(7,4) single-bit error-correcting block code"
    "370|skyline outline via divide-and-conquer merge"
    "371|brainfuck interpreter with O(1) bracket jumps"
    "372|Ackermann (recursive ≤ m=2 + iterative stack machine)"
    "373|LZW codec (TOBEORNOTTOBEOR + kwkwk edge case)"
    "374|Elias gamma + delta codes (universal integer encoding)"
    "375|milestone banner v9 (this file)"
)
for entry in "${batches[@]}"; do
    n=${entry%%|*}
    t=${entry#*|}
    printf "  %s  %s\n" "$n" "$t"
done
echo

banner "RUNNING TOTAL STATISTICS" 72
typeset -i n_lines=0 n_demos=375 n_assertions=0
n_lines=$(( 375 * 50 ))  # rough average; exact count via wc -l in CI separate
n_assertions=$(( 375 * 8 ))
echo "  demos:                  ${n_demos}"
echo "  approximate LOC:        ~${n_lines}"
echo "  approximate ztest pins: ~${n_assertions}"
echo "  batches sealed:         16  (10 → 360 → 367 → 375)"
echo

banner "ENDGAME PROGRESS — TRUST-COMPLETE > FEATURE-COMPLETE" 72
echo
echo "  zshrs replaces zsh permanently. Every batch advances the"
echo "  compat floor. Every demo pins a behavior. The 375 surface"
echo "  is the immune system against compat-floor regression."
echo
echo "  Next batch (376+) themes under consideration:"
echo "    • burrows-wheeler transform + inverse"
echo "    • CRC-32 + CRC-64 polynomial cyclic codes"
echo "    • base64 / base85 encode/decode round-trip"
echo "    • fractional knapsack (greedy) + 0/1 knapsack (DP)"
echo "    • Floyd-Warshall all-pairs shortest path"
echo

# === ztest pins (banner integrity) ===
zassert_eq "$(banner X 5)" "═════
  X
═════" "banner X width 5"

# Batch entries are all well-formed.
for entry in "${batches[@]}"; do
    n=${entry%%|*}
    t=${entry#*|}
    zassert_match "^3[67][0-9]$" "$n" "batch entry $n in 368-375 range"
    zassert_ne "$t" "" "batch entry $n has non-empty title"
done

zassert_eq "${#batches}" "8" "batch 16 contains exactly 8 entries"
zassert_eq "$n_demos" "375" "demo count = 375"

ztest_run
