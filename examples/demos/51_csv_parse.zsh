#!/usr/bin/env zshrs
# Simple CSV parsing — no embedded quotes/commas.

parse_row() {
    IFS=',' fields=( $=1 )
}

# Sample data fed via heredoc.
rows=()
while read -r line; do
    rows+=("$line")
done <<EOF
name,age,role
alice,30,admin
bob,25,user
carol,35,guest
dave,40,admin
eve,28,user
EOF

echo "── header ──"
parse_row "${rows[1]}"
header=("${fields[@]}")
echo "fields: ${header[@]}"

echo "── data rows ──"
for ((i = 2; i <= ${#rows[@]}; i++)); do
    parse_row "${rows[i]}"
    for ((j = 1; j <= ${#header[@]}; j++)); do
        printf "  %s=%s" "${header[j]}" "${fields[j]}"
    done
    echo
done

echo "── filter role=admin ──"
for ((i = 2; i <= ${#rows[@]}; i++)); do
    parse_row "${rows[i]}"
    if [[ ${fields[3]} == admin ]]; then
        printf "%s (age %s)\n" "${fields[1]}" "${fields[2]}"
    fi
done

echo "── total age ──"
total=0
for ((i = 2; i <= ${#rows[@]}; i++)); do
    parse_row "${rows[i]}"
    (( total += fields[2] ))
done
echo "sum of ages = $total"
echo "average = $(( total / (${#rows[@]} - 1) ))"
