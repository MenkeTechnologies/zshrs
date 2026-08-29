#!/usr/bin/env zshrs
# Associative arrays — comprehensive (kv) flag + zsh-specific patterns.
# Ports Src/params.c PM_HASHED + Src/subst.c paramsubst (kv) flag.

echo "── basic assoc declaration ──"
typeset -A colors
colors=(
    red   "#ff0000"
    green "#00ff00"
    blue  "#0000ff"
    cyan  "#00ffff"
    magenta "#ff00ff"
    yellow "#ffff00"
)

echo "  size: ${#colors}"
echo "  keys: ${(@k)colors}"
echo "  vals: ${(@v)colors}"

echo
echo "── access ──"
echo "  colors[red]:   ${colors[red]}"
echo "  colors[blue]:  ${colors[blue]}"
echo "  colors[noexist]: '${colors[noexist]:-none}'"

echo
echo "── (k) flag — list keys ──"
echo "  ${(@k)colors}"
echo "  sorted: ${(@ko)colors}"

echo
echo "── (v) flag — list values ──"
echo "  ${(@v)colors}"
echo "  sorted: ${(@vo)colors}"

echo
echo "── (kv) flag — key-value pairs as flat array ──"
echo "  ${(@kv)colors}"

echo
echo "── iterate as pairs ──"
for key val in "${(@kv)colors}"; do
    printf "  %-10s %s\n" "$key" "$val"
done

echo
echo "── set/get/delete operations ──"
colors[orange]="#ffa500"
echo "  added orange: size now ${#colors}"

unset 'colors[red]'
echo "  removed red: keys now ${(@ko)colors}"

# Bulk update.
colors+=( pink "#ffc0cb" white "#ffffff" black "#000000" )
echo "  bulk add: size now ${#colors}"

echo
echo '── ${+...+} existence check ──'
echo "  red exists: ${+colors[red]} (0=no, 1=yes)"
echo "  blue exists: ${+colors[blue]}"
echo "  unknown exists: ${+colors[unknown]}"

echo
echo "── parameter expansion flags on assoc ──"
echo "  count:          ${#colors}"
echo "  key/value:      ${(k)colors[blue]} (=blue) / ${colors[blue]}"
echo "  upper keys:     ${(U)${(@ko)colors}}"
# NB: `j` needs an argument (j:sep:); plain reverse-key order is (kO).
echo "  flag (kO) — keys in reverse sort order:"
echo "    ${(@kO)colors}"

echo
echo "── two-dim emulation via composite keys ──"
typeset -A matrix
for i in 1 2 3; do
    for j in 1 2 3; do
        matrix[$i,$j]=$(( i * 10 + j ))
    done
done
echo "  3x3 matrix:"
for ((i=1; i<=3; i++)); do
    for ((j=1; j<=3; j++)); do
        printf "    matrix[%d,%d] = %d\n" $i $j ${matrix[$i,$j]}
    done
done

echo
echo "── nested assoc via composite ──"
typeset -A users
users[alice.age]=30
users[alice.email]="alice@example.com"
users[alice.role]="admin"
users[bob.age]=25
users[bob.email]="bob@example.com"
users[bob.role]="user"

echo "  alice fields:"
for k in "${(@k)users}"; do
    if [[ $k == alice.* ]]; then
        local field=${k#alice.}
        printf "    %s = %s\n" "$field" "${users[$k]}"
    fi
done

echo
echo "── filter assoc by value pattern ──"
typeset -A scores
scores=(
    alice 95
    bob 72
    carol 88
    dave 65
    eve 91
)

echo "  high scorers (>= 85):"
for name in "${(@ko)scores}"; do
    s=${scores[$name]}
    if (( s >= 85 )); then
        printf "    %s: %d\n" "$name" "$s"
    fi
done

echo
echo "── invert (swap k/v) ──"
typeset -A inv
for k v in "${(@kv)colors}"; do
    inv[$v]=$k
done
echo "  forward: colors[orange] = ${colors[orange]}"
echo "  inverse: inv[#ffa500] = ${inv[#ffa500]}"

echo
echo "── merge two assocs ──"
typeset -A a b merged
a=( x 1 y 2 z 3 )
b=( y 20 z 30 w 40 )
merged=( "${(@kv)a}" "${(@kv)b}" )
echo "  a: ${(@kv)a}"
echo "  b: ${(@kv)b}"
echo "  merged: ${(@kv)merged}"
echo "  (later wins: y=20, z=30 from b)"

echo
echo "── (kvio) ordered flag combinations ──"
echo "  (ko) sorted keys:    ${(@ko)colors}"
echo "  (vo) sorted values:  ${(@vo)colors}"
echo "  (kvi) inverse-sort:  $(echo ${(@kOj: :)colors})"

echo
echo "── count distinct values ──"
typeset -A svc_status
svc_status=(
    server1 up
    server2 down
    server3 up
    server4 up
    server5 maintenance
    server6 down
)
typeset -A status_count
for v in "${(@v)svc_status}"; do
    (( status_count[$v]++ ))
done
for s in "${(@ko)status_count}"; do
    printf "  %-12s × %d\n" "$s" "${status_count[$s]}"
done

echo
echo "── (kv) flag in print -P / printf ──"
echo "  printf table:"
print -P "%-10s %s" Key Value
print -P "%-10s %s" "──────" "─────"
for k v in "${(@kv)colors}"; do
    printf "  %-10s %s\n" "$k" "$v"
done

echo
echo "── stats ──"
echo "  PM_HASHED implementation: Src/params.c"
echo "  paramsubst (kv) flag:     Src/subst.c (handle_zarrayflags)"
echo "  built-in support:         declare -A, typeset -A"
echo "  splay-tree vs hash:       zsh uses hash table"

# === ztest assertions ===
zassert_eq "${#colors}"          "9"        "9 colours after add/remove/bulk-add"
zassert_eq "${+colors[red]}"     "0"        "red was unset"
zassert_eq "${+colors[blue]}"    "1"        "blue is present"
zassert_eq "${colors[blue]}"     "#0000ff"  "blue value"
zassert_eq "${(j: :)${(@ko)colors}}" "black blue cyan green magenta orange pink white yellow" "sorted keys"
zassert_eq "${${(@kO)colors}[1]}"    "yellow"   "kO gives reverse-sorted keys"
zassert_eq "${svc_status[server1]}" "up"    "second assoc holds its own keys"
zassert_eq "${status_count[up]}"    "3"     "3 servers up"
ztest_run
