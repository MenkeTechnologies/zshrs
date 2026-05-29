#!/usr/bin/env zshrs
# Bank-account ledger — balance, deposit, withdraw, audit.

typeset -A BALANCE
typeset -a JOURNAL

acct_open() {
    local name=$1 init=${2:-0}
    BALANCE[$name]=$init
    JOURNAL+=("OPEN|$name|$init")
    echo "opened $name with $init"
}

acct_deposit() {
    local name=$1 amount=$2
    if [[ -z ${BALANCE[$name]+x} ]]; then
        echo "no such account: $name"
        return 1
    fi
    BALANCE[$name]=$(( ${BALANCE[$name]} + amount ))
    JOURNAL+=("DEP|$name|$amount")
    echo "  $name: deposit $amount → balance ${BALANCE[$name]}"
}

acct_withdraw() {
    local name=$1 amount=$2
    if [[ -z ${BALANCE[$name]+x} ]]; then
        echo "no such account: $name"
        return 1
    fi
    if (( ${BALANCE[$name]} < amount )); then
        echo "  $name: INSUFFICIENT (have ${BALANCE[$name]}, need $amount)"
        JOURNAL+=("WD-FAIL|$name|$amount")
        return 1
    fi
    BALANCE[$name]=$(( ${BALANCE[$name]} - amount ))
    JOURNAL+=("WD|$name|$amount")
    echo "  $name: withdraw $amount → balance ${BALANCE[$name]}"
}

acct_transfer() {
    local from=$1 to=$2 amount=$3
    if acct_withdraw "$from" "$amount"; then
        acct_deposit "$to" "$amount"
        JOURNAL+=("XFER|$from→$to|$amount")
    fi
}

audit() {
    echo "── audit ──"
    local total=0
    for name in ${(ko)BALANCE}; do
        printf "  %-12s %10d\n" $name ${BALANCE[$name]}
        (( total += BALANCE[$name] ))
    done
    printf "  %-12s %10d\n" "TOTAL" $total
}

journal() {
    echo "── journal (${#JOURNAL[@]} entries) ──"
    for entry in "${JOURNAL[@]}"; do
        echo "  $entry"
    done
}

echo "── operations ──"
acct_open alice 1000
acct_open bob 500
acct_open charlie 0

acct_deposit alice 200
acct_deposit bob 300

acct_withdraw alice 150
acct_withdraw charlie 50    # fails — empty

acct_transfer alice bob 100
acct_transfer alice charlie 250

audit
journal

echo "── post-audit total invariant ──"
total=0
for name in ${(k)BALANCE}; do
    (( total += BALANCE[$name] ))
done
echo "sum across accounts: $total"
