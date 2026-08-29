#!/usr/bin/env zshrs
# Inventory system — qty tracking with composite operations.

typeset -A QTY
typeset -A PRICE

add_item() {
    local name=$1 qty=$2 price=$3
    QTY[$name]=$(( ${QTY[$name]:-0} + qty ))
    PRICE[$name]=$price
}

remove_item() {
    local name=$1 qty=$2
    if (( ${QTY[$name]:-0} >= qty )); then
        QTY[$name]=$(( QTY[$name] - qty ))
        if (( QTY[$name] == 0 )); then
            unset "QTY[$name]"
        fi
        echo "removed $qty $name"
    else
        echo "insufficient: $name has ${QTY[$name]:-0}, requested $qty"
    fi
}

list_inventory() {
    if (( ${#QTY[@]} == 0 )); then
        echo "(empty)"
        return
    fi
    local total_value=0
    printf "%-12s %5s %8s %10s\n" "Item" "Qty" "Price" "Value"
    printf "%-12s %5s %8s %10s\n" "----" "---" "-----" "-----"
    for name in ${(ko)QTY}; do
        local q=${QTY[$name]}
        local p=${PRICE[$name]:-0}
        local v=$(( q * p ))
        (( total_value += v ))
        printf "%-12s %5d %8d %10d\n" $name $q $p $v
    done
    printf "%-12s %5s %8s %10d\n" "TOTAL" "" "" $total_value
}

restock_below() {
    local threshold=$1
    echo "items below $threshold:"
    for name in ${(ko)QTY}; do
        if (( QTY[$name] < threshold )); then
            printf "  %s: %d (restock!)\n" $name ${QTY[$name]}
        fi
    done
}

echo "── stock the store ──"
add_item apple 100 50
add_item banana 50 30
add_item cherry 200 100
add_item date 30 200
add_item elderberry 5 300

echo "── initial inventory ──"
list_inventory

echo "── sell some ──"
remove_item apple 20
remove_item banana 15
remove_item cherry 50

echo "── after sales ──"
list_inventory

echo "── try to oversell ──"
remove_item date 100

echo "── restock report ──"
restock_below 50

echo "── add more elderberry ──"
add_item elderberry 100 300

echo "── final inventory ──"
list_inventory

# === ztest assertions ===
# State below is what the demo body above leaves behind: the add_item totals
# minus every successful remove_item.
zassert_eq "${QTY[apple]}"      "80"   "apple stock = 100 added - 20 removed"
zassert_eq "${QTY[banana]}"     "35"   "banana stock = 50 added - 15 removed"
zassert_eq "${QTY[cherry]}"     "150"  "cherry stock = 200 added - 50 removed"
zassert_eq "${QTY[date]}"       "30"   "date stock = 30"
zassert_eq "${QTY[elderberry]}" "105"  "elderberry stock = 105 (5 + 100)"
zassert_eq "${PRICE[apple]}"    "50"   "apple price = 50"
zassert_eq "${PRICE[date]}"     "200"  "date price = 200"
zassert_eq "${#QTY[@]}"         "5"    "5 distinct items"
# Over-large request takes the insufficient branch and leaves stock untouched.
zassert_contains "$(remove_item apple 9999)" "insufficient" "remove_item: insufficient branch"
zassert_eq "${QTY[apple]}" "80"  "insufficient removal does not decrement"
zassert_contains "$(remove_item apple 30)" "removed 30 apple" "remove_item: success branch"
zassert_eq "${QTY[apple]}" "80"  'command substitution forks, so stock is unchanged here'
remove_item apple 30 >/dev/null   # same call in the current shell
zassert_eq "${QTY[apple]}" "50"  "successful in-shell removal decrements"
zassert_contains "$(restock_below 50)"     "date"  "restock_below 50 flags date"
zassert_contains "$(restock_below 100)"    "date" "restock_below 100 flags date"
ztest_run
