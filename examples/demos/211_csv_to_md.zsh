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

# === ztest assertions ===
t1=$(csv_to_md <<EOF
Name,Age,Role
Alice,30,Admin
EOF
)
zassert_contains "$t1" "| Name | Age | Role |" "t1 header row"
zassert_contains "$t1" "| --- | --- | --- |"  "t1 separator row"
zassert_contains "$t1" "| Alice | 30 | Admin |" "t1 data row"
t2=$(csv_to_md <<EOF
Col1,Col2,Col3
A,B,C
EOF
)
zassert_contains "$t2" "| A | B | C |" "t2 single data row"
zassert_eq $(echo "$t2" | wc -l | tr -d ' ') 3 "t2 has 3 lines (header,sep,data)"
ztest_run
