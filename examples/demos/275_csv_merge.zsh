#!/usr/bin/env zshrs
# CSV merge — join two CSVs on shared column.

tmpdir=$(mktemp -d)

cat > $tmpdir/users.csv <<'EOF'
id,name,email
1,Alice,alice@example.com
2,Bob,bob@example.com
3,Carol,carol@example.com
4,Dave,dave@example.com
5,Eve,eve@example.com
EOF

cat > $tmpdir/orders.csv <<'EOF'
order_id,user_id,amount,product
101,1,29.99,book
102,2,15.50,pen
103,1,99.00,headphones
104,3,5.25,sticker
105,2,250.00,monitor
106,5,12.75,mug
107,1,45.00,cable
EOF

echo "── source CSVs ──"
echo "users.csv:"
cat $tmpdir/users.csv | sed 's/^/  /'
echo
echo "orders.csv:"
cat $tmpdir/orders.csv | sed 's/^/  /'

echo
echo "── inner join (orders.user_id = users.id) ──"
# Build user index.
typeset -A user_name user_email
{
    IFS= read -r header
    while IFS=, read -r id name email; do
        user_name[$id]=$name
        user_email[$id]=$email
    done
} < $tmpdir/users.csv

# Walk orders, emit joined rows.
echo "  order_id,user_id,user_name,amount,product"
{
    IFS= read -r header
    while IFS=, read -r oid uid amt prod; do
        name=${user_name[$uid]:-UNKNOWN}
        echo "  $oid,$uid,$name,$amt,$prod"
    done
} < $tmpdir/orders.csv

echo
echo "── aggregate: total spent per user ──"
typeset -A totals counts
{
    IFS= read -r header
    while IFS=, read -r oid uid amt prod; do
        # Multiply by 100 to use int math.
        dollars=${amt%.*}
        cents=${amt#*.}
        cents="${cents}00"
        cents=${cents[1,2]}
        total_cents=$(( dollars * 100 + cents ))
        totals[$uid]=$(( ${totals[$uid]:-0} + total_cents ))
        (( counts[$uid]++ ))
    done
} < $tmpdir/orders.csv

format_cents() {
    printf "\$%d.%02d" $(( $1 / 100 )) $(( $1 % 100 ))
}

echo "  uid name        orders   total"
for uid in "${(@ko)totals}"; do
    name=${user_name[$uid]:-UNKNOWN}
    printf "  %3s %-10s %5d   %s\n" "$uid" "$name" "${counts[$uid]}" "$(format_cents ${totals[$uid]})"
done

echo
echo "── unmatched users (no orders) ──"
{
    IFS= read -r header
    while IFS=, read -r id name email; do
        if (( ! ${+totals[$id]} )); then
            echo "  $id $name $email"
        fi
    done
} < $tmpdir/users.csv

echo
echo "── full outer join (all users + all orders, NULL fills) ──"
echo "  user_id,name,order_id,amount"

# Users with orders.
{
    IFS= read -r header
    declare -A printed_users
    while IFS=, read -r oid uid amt prod; do
        name=${user_name[$uid]:-NULL}
        echo "  $uid,$name,$oid,$amt"
        printed_users[$uid]=1
    done
} < $tmpdir/orders.csv

# Users without orders (still need to be shown).
{
    IFS= read -r header
    while IFS=, read -r id name email; do
        if (( ! ${+totals[$id]} )); then
            echo "  $id,$name,NULL,NULL"
        fi
    done
} < $tmpdir/users.csv

echo
echo "── top spender ──"
top_uid=""
top_total=0
for uid in "${(@ko)totals}"; do
    if (( totals[$uid] > top_total )); then
        top_total=${totals[$uid]}
        top_uid=$uid
    fi
done
echo "  user $top_uid (${user_name[$top_uid]}) — $(format_cents $top_total)"

# === ztest assertions ===
zassert_eq "${user_name[1]}"   "Alice"  "user 1 = Alice"
zassert_eq "${user_name[5]}"   "Eve"    "user 5 = Eve"
zassert_eq "${user_email[2]}"  "bob@example.com" "user 2 email"
zassert_eq "${counts[1]}"      3        "Alice has 3 orders"
zassert_eq "${counts[2]}"      2        "Bob has 2 orders"
zassert_eq "${counts[3]}"      1        "Carol has 1 order"
zassert_eq "${counts[5]}"      1        "Eve has 1 order"
zassert_eq "${totals[1]}"      17399    "Alice total cents = 29.99 + 99.00 + 45.00 = 173.99"
zassert_eq "${totals[2]}"      26550    "Bob total cents = 15.50 + 250.00 = 265.50"
zassert_eq "$top_uid"          2        "Bob is top spender"
zassert_eq "$top_total"        26550    "top total = 26550 cents"
zassert_eq "$(format_cents 17399)" '$173.99' "format_cents formats correctly"

command rm -rf $tmpdir
ztest_run
