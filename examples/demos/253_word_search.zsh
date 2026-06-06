#!/usr/bin/env zshrs
# Word search grid solver — find words in 8 directions.
# Uses precomputed flat grid for speed (no function call per cell).

# Grid as space-sep rows; collapse to flat string for fast indexing.
typeset -a GRID
GRID=(
    "Z S H R S R U S T"
    "B Y T E C O D E I"
    "F U S E V M S Z W"
    "S H E L L O W L Q"
    "F O R K E X E C N"
    "C O M P I L E R S"
    "J I T C A C H E P"
    "P A R A L L E L A"
    "E X P A N S I O N"
)

ROWS=${#GRID}
COLS=9

# Build flat letter string (no spaces) for fast indexing.
FLAT=""
for row in "${GRID[@]}"; do
    FLAT+="${row// /}"
done

# Direction deltas as flat array of pairs.
typeset -a DR DC
DR=( 0  0  1 -1  1 -1  1 -1)
DC=( 1 -1  0  0  1 -1 -1  1)
DIR_NAMES=(E W S N SE NW SW NE)

find_word() {
    local word=${1:u}
    local len=${#word}
    local r c d dr dc i ok cr cc letter
    for ((r=1; r<=ROWS; r++)); do
        for ((c=1; c<=COLS; c++)); do
            for ((d=1; d<=8; d++)); do
                dr=${DR[d]}
                dc=${DC[d]}
                ok=1
                for ((i=0; i<len; i++)); do
                    cr=$((r + i*dr))
                    cc=$((c + i*dc))
                    if (( cr < 1 || cr > ROWS || cc < 1 || cc > COLS )); then
                        ok=0; break
                    fi
                    local fidx=$(( (cr-1)*COLS + cc ))
                    letter=${FLAT[fidx]}
                    if [[ ${letter:u} != ${word[i+1]} ]]; then
                        ok=0; break
                    fi
                done
                if (( ok )); then
                    printf "  ✓ '%s' at (%d,%d) dir=%s\n" "$word" $r $c "${DIR_NAMES[d]}"
                    return 0
                fi
            done
        done
    done
    printf "  ✗ '%s' not found\n" "$word"
    return 1
}

echo "── grid (${ROWS}x${COLS}) ──"
for ((r=1; r<=ROWS; r++)); do
    echo "  ${GRID[r]}"
done
echo "  flat: $FLAT"

echo
echo "── search words ──"
words=(ZSHRS RUST FORK EXEC JIT CACHE NOPE PYTHON)
found=0
total=${#words}
for w in "${words[@]}"; do
    if find_word "$w"; then
        (( found++ ))
    fi
done

echo
echo "── stats ──"
echo "  found: $found / $total"
echo "  hit rate: $(( found * 100 / total ))%"

# === ztest assertions ===
zassert_eq "$ROWS" "9"  "9 rows"
zassert_eq "$COLS" "9"  "9 columns"
zassert_eq "${#FLAT}" "81" "flat string is 9x9 = 81 chars"
zassert_eq "${FLAT[1,5]}" "ZSHRS" "row 1 starts ZSHRS"
zassert_contains "$FLAT" "RUST"  "FLAT contains RUST"
zassert_contains "$FLAT" "FORK"  "FLAT contains FORK"
zassert_contains "$FLAT" "CACHE" "FLAT contains CACHE"
zassert_eq "$found" "6" "6 words found"
zassert_eq "$total" "8" "8 words searched"
if find_word ZSHRS  >/dev/null; then zassert_ok 1 "ZSHRS findable";  else zassert_ok 0 "ZSHRS findable"; fi
if find_word PYTHON >/dev/null; then zassert_ok 0 "PYTHON not in grid"; else zassert_ok 1 "PYTHON not in grid"; fi
ztest_run
