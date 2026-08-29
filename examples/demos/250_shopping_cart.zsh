#!/usr/bin/env zshrs
# Shopping cart — items, prices, quantities, tax + discounts.

typeset -A PRICES INVENTORY
typeset -A CART

# Stock catalog.
PRICES=(
    apple 0.50
    bread 2.99
    milk 3.49
    eggs 4.99
    cheese 7.99
    coffee 12.49
    tea 6.99
    rice 8.99
    pasta 1.99
    sauce 3.79
)

INVENTORY=(
    apple 100
    bread 20
    milk 15
    eggs 30
    cheese 12
    coffee 8
    tea 25
    rice 50
    pasta 40
    sauce 22
)

# Add to cart, checking inventory.
add_to_cart() {
    local item=$1 qty=$2
    local stock=${INVENTORY[$item]:-0}
    if (( qty > stock )); then
        echo "  ✗ insufficient stock: requested $qty $item, have $stock"
        return 1
    fi
    if (( ! ${+PRICES[$item]} )); then
        echo "  ✗ unknown item: $item"
        return 1
    fi
    CART[$item]=$(( ${CART[$item]:-0} + qty ))
    INVENTORY[$item]=$(( stock - qty ))
    printf "  ✓ +%d %s @ \$%s (now %d in cart)\n" $qty $item ${PRICES[$item]} ${CART[$item]}
}

# Remove from cart.
remove_from_cart() {
    local item=$1 qty=$2
    local cur=${CART[$item]:-0}
    if (( qty > cur )); then
        echo "  ✗ only $cur $item in cart"
        return 1
    fi
    CART[$item]=$(( cur - qty ))
    INVENTORY[$item]=$(( ${INVENTORY[$item]:-0} + qty ))
    [[ ${CART[$item]} == 0 ]] && unset "CART[$item]"
    printf "  ✓ -%d %s (remaining: %d)\n" $qty $item ${CART[$item]:-0}
}

# Compute subtotal in cents (avoid float).
subtotal_cents() {
    local total=0 item qty cents
    for item in "${(@k)CART}"; do
        qty=${CART[$item]}
        # Convert "X.YZ" to cents.
        local price=${PRICES[$item]}
        local dollars=${price%.*}
        local frac=${price#*.}
        # Pad to 2.
        frac="${frac}00"
        frac=${frac[1,2]}
        cents=$(( dollars * 100 + frac ))
        (( total += cents * qty ))
    done
    echo $total
}

format_cents() {
    local c=$1
    printf "\$%d.%02d" $((c / 100)) $((c % 100))
}

print_cart() {
    echo "── cart ──"
    if (( ${#CART} == 0 )); then echo "  (empty)"; return; fi
    local item
    for item in "${(@ko)CART}"; do
        printf "  %-10s x %3d @ %s\n" "$item" "${CART[$item]}" "$(format_cents $(( $(price_cents $item) )))"
    done
}

price_cents() {
    local p=${PRICES[$1]}
    local d=${p%.*} f=${p#*.}
    f="${f}00"; f=${f[1,2]}
    echo $(( d * 100 + f ))
}

apply_tax() {
    local subtotal=$1 rate_pct=${2:-825}  # 8.25% expressed as 825 / 10000
    echo $(( subtotal * rate_pct / 10000 ))
}

apply_discount() {
    local subtotal=$1 pct=$2
    echo $(( subtotal * pct / 100 ))
}

echo "── add items ──"
add_to_cart apple 6
add_to_cart bread 2
add_to_cart milk 1
add_to_cart cheese 1
add_to_cart coffee 1
add_to_cart pasta 4
add_to_cart sauce 2

echo
print_cart

echo
echo "── remove item ──"
remove_from_cart apple 2

echo
echo "── try invalid ──"
add_to_cart coffee 999     # stock fail
add_to_cart unicorn 1       # unknown

echo
print_cart

echo
echo "── totals ──"
sub=$(subtotal_cents)
echo "  subtotal:  $(format_cents $sub)"
tax=$(apply_tax $sub 825)
echo "  tax (8.25%): $(format_cents $tax)"
disc=$(apply_discount $sub 10)
echo "  discount (10%): -$(format_cents $disc)"
final=$(( sub + tax - disc ))
echo "  final:     $(format_cents $final)"

echo
echo "── inventory after checkout ──"
for item in "${(@ko)CART}"; do
    printf "  %-10s remaining stock: %d\n" "$item" "${INVENTORY[$item]}"
done

# === ztest assertions ===
zassert_eq "${PRICES[apple]}"     "0.50"  "PRICES[apple]"
zassert_eq "${PRICES[bread]}"     "2.99"  "PRICES[bread]"
zassert_eq "${PRICES[coffee]}"    "12.49" "PRICES[coffee]"
zassert_eq "${#PRICES[@]}"        "10"    "10 catalog items"
# Stock reflects the demo body above: 6 apples and 1 coffee left the shelf,
# and the coffee-999 / unicorn attempts were both refused.
zassert_eq "${INVENTORY[apple]}"  "96"    "INVENTORY[apple] after 6 sold"
zassert_eq "${INVENTORY[coffee]}" "7"     "INVENTORY[coffee] after 1 sold"
zassert_eq "${CART[apple]}"       "4"     "CART[apple] (6 added, 2 removed)"
zassert_eq "${CART[coffee]}"      "1"     "CART[coffee] (999 request refused)"
zassert_eq "${CART[unicorn]}"     ""      "unknown item never enters the cart"
zassert_eq "$(price_cents apple)" "50"    "apple = 50 cents"
zassert_eq "$(price_cents bread)" "299"   "bread = 299 cents"
zassert_eq "$(price_cents coffee)" "1249" "coffee = 1249 cents"
zassert_eq "$(format_cents 1234)" "\$12.34" "format_cents 1234"
zassert_eq "$(format_cents 5)"    "\$0.05"  "format_cents 5 (pad)"
zassert_eq "$(apply_tax 10000 825)" "825"  "8.25% tax on \$100"
zassert_eq "$(apply_discount 10000 10)" "1000" "10% off on \$100"
ztest_run
