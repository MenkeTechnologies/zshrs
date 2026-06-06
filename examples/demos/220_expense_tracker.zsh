#!/usr/bin/env zshrs
# Expense tracker — categorized transactions + summaries.

typeset -A TOTALS
typeset -a TRANSACTIONS

add_expense() {
    local date=$1 cat=$2 amount=$3 desc=$4
    TRANSACTIONS+=("$date|$cat|$amount|$desc")
    (( TOTALS[$cat] += amount ))
}

echo "── recording expenses ──"
add_expense "2026-05-01" food 12 "groceries"
add_expense "2026-05-02" food 8 "lunch"
add_expense "2026-05-03" transport 25 "uber"
add_expense "2026-05-04" entertainment 50 "concert"
add_expense "2026-05-05" food 20 "dinner"
add_expense "2026-05-06" transport 15 "gas"
add_expense "2026-05-07" rent 1200 "monthly rent"
add_expense "2026-05-08" utilities 80 "electric"
add_expense "2026-05-09" utilities 40 "water"
add_expense "2026-05-10" food 35 "groceries"

echo "── all transactions ──"
for t in "${TRANSACTIONS[@]}"; do
    set -- $=t
    printf "  %s | %-12s | \$%5d | %s\n" "${1//|/ }" "${(P)2}" "$3" "${(j: :)@:4}"
done | head -10

# Reparse simpler.
echo
echo "── transactions (clean) ──"
for t in "${TRANSACTIONS[@]}"; do
    local -a parts=( ${(s/|/)t} )
    printf "  %s | %-13s | \$%4d | %s\n" ${parts[1]} ${parts[2]} ${parts[3]} ${parts[4]}
done

echo
echo "── totals by category ──"
for cat in ${(ko)TOTALS}; do
    printf "  %-13s \$%d\n" $cat ${TOTALS[$cat]}
done

echo
echo "── grand total ──"
grand=0
for v in ${(v)TOTALS}; do (( grand += v )); done
echo "  TOTAL: \$$grand"

echo
echo "── percentages ──"
for cat in ${(ko)TOTALS}; do
    pct=$(( TOTALS[$cat] * 100 / grand ))
    printf "  %-13s %3d%% (\$%d)\n" $cat $pct ${TOTALS[$cat]}
done

echo
echo "── filter by category ──"
filter_cat() {
    local target=$1
    for t in "${TRANSACTIONS[@]}"; do
        local -a parts=( ${(s/|/)t} )
        if [[ ${parts[2]} == $target ]]; then
            printf "  %s \$%d %s\n" ${parts[1]} ${parts[3]} ${parts[4]}
        fi
    done
}

echo "food only:"
filter_cat food

echo
echo "── largest expense ──"
max_amount=0; max_desc=""
for t in "${TRANSACTIONS[@]}"; do
    local -a parts=( ${(s/|/)t} )
    if (( parts[3] > max_amount )); then
        max_amount=${parts[3]}
        max_desc="${parts[2]} ${parts[4]}"
    fi
done
echo "  \$$max_amount: $max_desc"

echo
echo "── daily total range ──"
first=${TRANSACTIONS[1]%%|*}
last=${TRANSACTIONS[-1]%%|*}
echo "  from $first to $last"
echo "  transactions: ${#TRANSACTIONS[@]}"

# === ztest assertions ===
zassert_eq "${#TRANSACTIONS[@]}" 10    "10 transactions recorded"
zassert_eq "${TOTALS[food]}"      75   "food total = 12+8+20+35"
zassert_eq "${TOTALS[transport]}" 40   "transport total = 25+15"
zassert_eq "${TOTALS[rent]}"      1200 "rent total"
zassert_eq "${TOTALS[utilities]}" 120  "utilities total"
zassert_eq "${TOTALS[entertainment]}" 50 "entertainment total"
zassert_eq "$grand"        1485        "grand total"
zassert_eq "$max_amount"   1200        "largest expense amount"
zassert_eq "$max_desc"     "rent monthly rent" "largest expense desc"
zassert_eq "$first"        "2026-05-01" "first txn date"
zassert_eq "$last"         "2026-05-10" "last txn date"
ztest_run
