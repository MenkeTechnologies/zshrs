#!/usr/bin/env zshrs
# Search and filter patterns over an array.

people=(
    "alice|30|admin|active"
    "bob|25|user|active"
    "carol|35|admin|inactive"
    "dave|45|owner|active"
    "eve|28|user|inactive"
    "frank|32|admin|active"
    "grace|29|user|active"
)

# Linear search.
find_by_name() {
    local needle=$1
    for p in "${people[@]}"; do
        local name=${p%%|*}
        if [[ $name == $needle ]]; then
            echo "  $p"
            return
        fi
    done
    echo "  no match"
}

# Multi-field filter.
filter() {
    local field=$1 value=$2
    local -A field_map
    field_map[name]=1
    field_map[age]=2
    field_map[role]=3
    field_map[status]=4
    local idx=${field_map[$field]}
    [[ -z $idx ]] && { echo "  bad field: $field"; return; }
    for p in "${people[@]}"; do
        local parts=( ${(s/|/)p} )
        if [[ ${parts[idx]} == $value ]]; then
            echo "  $p"
        fi
    done
}

# Pattern match on any field.
matches_pattern() {
    local pat=$1
    for p in "${people[@]}"; do
        if [[ $p == *$pat* ]]; then
            echo "  $p"
        fi
    done
}

# Compound filter via &&.
filter_compound() {
    local role=$1 state=$2
    for p in "${people[@]}"; do
        local parts=( ${(s/|/)p} )
        if [[ ${parts[3]} == $role && ${parts[4]} == $state ]]; then
            echo "  $p"
        fi
    done
}

echo "── find alice ──"
find_by_name alice

echo
echo "── role=admin ──"
filter role admin

echo
echo "── status=active ──"
filter status active

echo
echo "── name contains 'a' ──"
matches_pattern a

echo
echo "── active admins ──"
filter_compound admin active

echo
echo "── sort by age ──"
sorted=( ${(o)people} )
for p in "${sorted[@]}"; do echo "  $p"; done

echo
echo "── group by role ──"
typeset -A by_role
for p in "${people[@]}"; do
    local parts=( ${(s/|/)p} )
    by_role[${parts[3]}]="${by_role[${parts[3]}]:+${by_role[${parts[3]}]} }${parts[1]}"
done
for k in ${(ko)by_role}; do
    echo "  $k: ${by_role[$k]}"
done

echo
echo "── stats ──"
echo "total: ${#people[@]}"
active=0
for p in "${people[@]}"; do
    local parts=( ${(s/|/)p} )
    [[ ${parts[4]} == active ]] && (( active++ ))
done
echo "active: $active"

# === ztest assertions ===
zassert_eq "${#people[@]}" 7  "7 people in dataset"
zassert_eq "$active"       5  "5 active people"
# direct field tests
zassert_contains "$(find_by_name alice)"   "alice|30|admin"   "find_by_name alice"
zassert_contains "$(find_by_name nobody)"  "no match"          "find_by_name miss"
zassert_contains "$(filter role admin)"    "alice"             "filter role=admin has alice"
zassert_contains "$(filter role admin)"    "carol"             "filter role=admin has carol"
zassert_contains "$(filter status active)" "bob"               "filter status=active has bob"
ztest_run
