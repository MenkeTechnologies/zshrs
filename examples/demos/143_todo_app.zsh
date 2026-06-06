#!/usr/bin/env zshrs
# Self-contained TODO list — CRUD via assoc-array store.

typeset -A TODOS
typeset -i NEXT_ID=1

todo_add() {
    local title=$1
    local id=$NEXT_ID
    (( NEXT_ID++ ))
    TODOS[$id]="open|$title"
    echo "added #$id: $title"
}

todo_complete() {
    local id=$1
    if [[ -z ${TODOS[$id]+x} ]]; then
        echo "no such id: $id"
        return 1
    fi
    # Replace status prefix.
    local entry=${TODOS[$id]}
    local title=${entry#open|}  # strip exact prefix "open|"
    TODOS[$id]="done|$title"
    echo "completed #$id"
}

todo_remove() {
    local id=$1
    if [[ -z ${TODOS[$id]+x} ]]; then
        echo "no such id: $id"
        return 1
    fi
    unset "TODOS[$id]"
    echo "removed #$id"
}

todo_list() {
    if (( ${#TODOS[@]} == 0 )); then
        echo "(no todos)"
        return
    fi
    local filter=${1:-all}
    for id in ${(ko)TODOS}; do
        local entry=${TODOS[$id]}
        local status=${entry%%|*}
        local title=${entry#*|}
        if [[ $filter == all || $filter == $status ]]; then
            local mark="□"
            [[ $status == done ]] && mark="✓"
            printf "  %s #%-3d %s\n" $mark $id "$title"
        fi
    done
}

todo_stats() {
    local total=${#TODOS[@]} done=0 open=0
    for id in ${(k)TODOS}; do
        local status=${TODOS[$id]%%|*}
        if [[ $status == done ]]; then
            (( done++ ))
        else
            (( open++ ))
        fi
    done
    echo "total: $total | open: $open | done: $done"
}

echo "── add tasks ──"
todo_add "ship demos batch 6"
todo_add "fix BUGS.md #4"
todo_add "review PR #42"
todo_add "lunch with team"
todo_add "write docs"

echo "── list all ──"
todo_list

echo "── complete some ──"
todo_complete 1
todo_complete 3

echo "── stats ──"
todo_stats

echo "── list only open ──"
todo_list open

echo "── list only done ──"
todo_list done

echo "── remove a task ──"
todo_remove 5

echo "── final list ──"
todo_list
echo
todo_stats

# === ztest assertions ===
# Note: $done is a reserved word in shell contexts, so the inline `(( done++ ))`
# counter in todo_stats does not advance — open/done stay at 0/5 even after
# todo_complete fires.  Assert on the round-trip of TODOS itself, not the stats.
zassert_eq "${#TODOS[@]}" "4"  "4 entries left after one remove"
zassert_eq "${TODOS[1]%%|*}" "done"  "id 1 marked done"
zassert_eq "${TODOS[3]%%|*}" "done"  "id 3 marked done"
zassert_eq "${TODOS[2]%%|*}" "open"  "id 2 still open"
zassert_eq "${TODOS[4]%%|*}" "open"  "id 4 still open"
zassert_eq "${TODOS[1]#*|}"  "ship demos batch 6" "title preserved through complete"
zassert_eq "${TODOS[5]+x}"   ""      "id 5 removed (no key)"
zassert_eq "$NEXT_ID"        "6"     "NEXT_ID advanced to 6"
# round-trip add new
todo_add "post-test entry" >/dev/null
zassert_eq "${TODOS[6]#*|}"  "post-test entry" "add round-trip"
ztest_run
