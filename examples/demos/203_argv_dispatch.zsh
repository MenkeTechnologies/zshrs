#!/usr/bin/env zshrs
# CLI subcommand dispatcher pattern.

typeset -A COMMANDS
COMMANDS[init]=cmd_init
COMMANDS[add]=cmd_add
COMMANDS[remove]=cmd_remove
COMMANDS[list]=cmd_list
COMMANDS[show]=cmd_show
COMMANDS[help]=cmd_help

typeset -A HELP_TEXT
HELP_TEXT[init]="Initialize a new project"
HELP_TEXT[add]="Add an item (args: <name> <value>)"
HELP_TEXT[remove]="Remove an item (args: <name>)"
HELP_TEXT[list]="List all items"
HELP_TEXT[show]="Show details for an item (args: <name>)"
HELP_TEXT[help]="Show this help message"

typeset -A STORE

cmd_init() {
    STORE=()
    echo "  initialized empty store"
}

cmd_add() {
    local name=$1 val=$2
    if [[ -z $name || -z $val ]]; then
        echo "  add: missing name or value"
        return 1
    fi
    STORE[$name]=$val
    echo "  added: $name = $val"
}

cmd_remove() {
    local name=$1
    if [[ -z ${STORE[$name]+x} ]]; then
        echo "  remove: no such item: $name"
        return 1
    fi
    unset "STORE[$name]"
    echo "  removed: $name"
}

cmd_list() {
    if (( ${#STORE[@]} == 0 )); then
        echo "  (empty)"
        return
    fi
    for k in ${(ko)STORE}; do
        printf "  %-12s = %s\n" $k ${STORE[$k]}
    done
}

cmd_show() {
    local name=$1
    if [[ -z ${STORE[$name]+x} ]]; then
        echo "  show: no such item: $name"
        return 1
    fi
    echo "  $name = ${STORE[$name]}"
}

cmd_help() {
    echo "  Available subcommands:"
    for k in ${(ko)COMMANDS}; do
        printf "    %-10s %s\n" $k "${HELP_TEXT[$k]}"
    done
}

dispatch() {
    local sub=$1
    shift
    if [[ -z $sub ]]; then
        echo "usage: program <subcommand> [args...]"
        cmd_help
        return 1
    fi
    if [[ -z ${COMMANDS[$sub]+x} ]]; then
        echo "  unknown subcommand: $sub"
        cmd_help
        return 1
    fi
    ${COMMANDS[$sub]} "$@"
}

echo "── help ──"
dispatch help

echo
echo "── init ──"
dispatch init

echo
echo "── add some items ──"
dispatch add apple 100
dispatch add banana 50
dispatch add cherry 200

echo
echo "── list ──"
dispatch list

echo
echo "── show single ──"
dispatch show banana

echo
echo "── remove ──"
dispatch remove banana

echo
echo "── list after remove ──"
dispatch list

echo
echo "── error cases ──"
dispatch unknown_cmd
dispatch remove nope
dispatch show absent
dispatch add  # missing args

# === ztest assertions ===
# After the script's flow: cherry and apple remain; banana was removed
zassert_eq "${#STORE[@]}" 2                   "2 items left after remove"
zassert_eq "${STORE[apple]}"  "100"           "apple kept"
zassert_eq "${STORE[cherry]}" "200"           "cherry kept"
zassert_eq "${STORE[banana]:-(missing)}" "(missing)"  "banana removed"
# Dispatcher round-trips
zassert_eq "${#COMMANDS[@]}" 6                "6 subcommands"
zassert_eq "${COMMANDS[init]}"  "cmd_init"    "init dispatch"
zassert_contains "$(dispatch list)" "apple"   "list shows apple"
zassert_contains "$(dispatch help)" "Available subcommands" "help banner"
# Add a new item via dispatch
dispatch add date 42 > /dev/null
zassert_eq "${STORE[date]}" "42"              "dispatched add works"
ztest_run
