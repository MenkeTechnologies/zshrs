#!/usr/bin/env zshrs
# Todo with categories + priorities + due dates.

typeset -a TODOS  # entries: id|priority|category|due|done|title

NEXT_ID=1

add() {
    local pri=$1 cat=$2 due=$3 title=$4
    TODOS+=("${NEXT_ID}|$pri|$cat|$due|open|$title")
    (( NEXT_ID++ ))
}

complete_task() {
    local id=$1
    for ((i=1; i<=${#TODOS[@]}; i++)); do
        local parts=( ${(s/|/)TODOS[i]} )
        if [[ ${parts[1]} == $id ]]; then
            parts[5]=done
            TODOS[i]="${(j:|:)parts}"
            echo "completed #$id"
            return
        fi
    done
    echo "no such id: $id"
}

list_filtered() {
    local key=$1 val=$2
    local idx_map=( id priority category due status title )
    for entry in "${TODOS[@]}"; do
        local parts=( ${(s/|/)entry} )
        local match=0
        for ((i=1; i<=${#idx_map[@]}; i++)); do
            if [[ ${idx_map[i]} == $key ]]; then
                [[ ${parts[i]} == $val ]] && match=1
                break
            fi
        done
        if [[ -z $key || $match == 1 ]]; then
            local status_icon="□"
            [[ ${parts[5]} == done ]] && status_icon="✓"
            printf "  %s #%-3d [%s] %-12s due=%s — %s\n" \
                $status_icon ${parts[1]} ${parts[2]} ${parts[3]} ${parts[4]} ${parts[6]}
        fi
    done
}

stats() {
    local total=${#TODOS[@]} done_=0 open_=0
    typeset -A by_cat by_pri
    for entry in "${TODOS[@]}"; do
        local parts=( ${(s/|/)entry} )
        if [[ ${parts[5]} == done ]]; then (( done_++ ))
        else (( open_++ ))
        fi
        (( by_cat[${parts[3]}]++ ))
        (( by_pri[${parts[2]}]++ ))
    done
    echo "total=$total open=$open_ done=$done_"
    echo "by category:"
    for k in ${(ko)by_cat}; do echo "  $k: ${by_cat[$k]}"; done
    echo "by priority:"
    for k in ${(ko)by_pri}; do echo "  $k: ${by_pri[$k]}"; done
}

echo "── add tasks ──"
add high   work     2026-05-30 "ship batch 9"
add high   personal 2026-06-01 "call dentist"
add medium work     2026-06-05 "review PRs"
add low    work     2026-06-10 "clean inbox"
add medium personal 2026-06-15 "meal prep"
add high   urgent   2026-05-29 "fix bug #42"
add low    learn    2026-07-01 "read book"

echo "── all ──"
list_filtered

echo
echo "── high priority only ──"
list_filtered priority high

echo
echo "── work category ──"
list_filtered category work

echo
echo "── complete some ──"
complete_task 1
complete_task 3
complete_task 6

echo
echo "── after completion ──"
list_filtered

echo
echo "── stats ──"
stats

# === ztest assertions ===
zassert_eq "${#TODOS[@]}" 7   "7 todos added"
zassert_eq "$NEXT_ID"     8   "NEXT_ID after 7 adds"
# task 1 completed
parts1=( ${(s/|/)TODOS[1]} )
zassert_eq "${parts1[5]}" "done" "task 1 status=done"
# task 2 still open
parts2=( ${(s/|/)TODOS[2]} )
zassert_eq "${parts2[5]}" "open" "task 2 status=open"
# task 6 completed
parts6=( ${(s/|/)TODOS[6]} )
zassert_eq "${parts6[5]}" "done" "task 6 status=done"
# stats output
stats_out=$(stats)
zassert_contains "$stats_out" "total=7" "stats shows total=7"
zassert_contains "$stats_out" "done=3"  "stats shows done=3"
zassert_contains "$stats_out" "open=4"  "stats shows open=4"
zassert_contains "$stats_out" "work: 3" "stats shows work: 3"
ztest_run
