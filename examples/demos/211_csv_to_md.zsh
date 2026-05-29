#!/usr/bin/env zshrs
# CSV → Markdown table converter.

csv_to_md() {
    local first=1
    while IFS=, read -r -A fields; do
        printf "|"
        for f in "${fields[@]}"; do printf " %s |" "$f"; done
        printf "\n"
        if (( first )); then
            printf "|"
            for f in "${fields[@]}"; do printf " --- |"; done
            printf "\n"
            first=0
        fi
    done
}

echo "── table 1 ──"
csv_to_md <<EOF
Name,Age,Role
Alice,30,Admin
Bob,25,User
Carol,35,Guest
EOF

echo
echo "── table 2 (data only) ──"
csv_to_md <<EOF
ID,Status,Score
1,active,92
2,active,85
3,inactive,73
4,active,98
EOF

echo
echo "── single row + headers ──"
csv_to_md <<EOF
Col1,Col2,Col3
A,B,C
EOF
