#!/usr/bin/env zshrs
# $argv, $#, $*, $@, $1..$9, shift — positional parameter mechanics.
# Ports Src/params.c $argv handling + Src/exec.c shift.

dump_args() {
    echo "  inside dump_args:"
    echo "    \$# = $#"
    echo "    \$@ = $@"
    echo "    \$* = $*"
    echo "    \$0 = $0"
    echo "    \$1 = $1"
    echo "    \$2 = $2"
    echo "    \$3 = $3"
    echo "    \$argv = $argv"
    echo "    \${#argv} = ${#argv}"
}

echo "── basic positional access ──"
dump_args alpha beta gamma delta epsilon

echo
echo "── shift mechanics ──"
shift_demo() {
    echo "  initial: \$# = $# ; \$@ = $@"
    shift
    echo "  after shift 1: \$# = $# ; \$@ = $@"
    shift 2
    echo "  after shift 2: \$# = $# ; \$@ = $@"
}
shift_demo a b c d e f g

echo
echo "── set -- to reset positional params ──"
reset_demo() {
    set -- new1 new2 new3
    echo "  after set --: \$# = $# ; \$@ = $@"
}
reset_demo orig1 orig2 orig3 orig4

echo
echo "── \"\$@\" vs \"\$*\" splitting ──"
split_demo() {
    echo "  \"\$@\" iter:"
    local i=0
    for a in "$@"; do
        (( i++ ))
        echo "    [$i] = '$a'"
    done
    echo "  \"\$*\" iter:"
    i=0
    for a in "$*"; do
        (( i++ ))
        echo "    [$i] = '$a'"
    done
}
split_demo "hello world" "foo bar" "baz"

echo
echo "── $argv array manipulation ──"
mod_argv() {
    echo "  argv before: ${argv[@]}"
    argv[1]="MODIFIED"
    echo "  argv after [1]=MODIFIED: ${argv[@]}"
    argv+=("appended")
    echo "  after append: ${argv[@]}"
}
mod_argv x y z

echo
echo "── \$0 in different contexts ──"
echo "  top-level \$0 = $0"
nested0() {
    echo "  inside nested0() \$0 = $0"
    deep_nest() {
        echo "  inside deep_nest() \$0 = $0"
    }
    deep_nest
}
nested0

echo
echo "── slicing positional params ──"
slice_demo() {
    echo "  \$@ = $@"
    echo "  \${@:1:3} = ${@:1:3}"
    echo "  \${@:2:2} = ${@:2:2}"
    echo "  \${@:3} = ${@:3}"
    echo "  \${argv[2,4]} = ${argv[2,4]}"
}
slice_demo a1 a2 a3 a4 a5 a6

echo
echo "── shift to consume args ──"
consume() {
    while (( $# > 0 )); do
        echo "  consume: $1"
        shift
    done
}
consume one two three four

echo
echo "── default values via \${var:-default} ──"
default_demo() {
    echo "  \${1:-DEFAULT}     = ${1:-DEFAULT}"
    echo "  \${2:-otherwise}   = ${2:-otherwise}"
    echo "  \${3:-FALLBACK}    = ${3:-FALLBACK}"
}
default_demo "first" "second"

echo
echo "── getopts iteration ──"
getopts_demo() {
    OPTIND=1
    while getopts "abc:d:" opt; do
        case $opt in
            a) echo "  -a flag" ;;
            b) echo "  -b flag" ;;
            c) echo "  -c arg = $OPTARG" ;;
            d) echo "  -d arg = $OPTARG" ;;
            *) echo "  unknown: $opt" ;;
        esac
    done
    shift $((OPTIND - 1))
    echo "  remaining \$@ = $@"
}
getopts_demo -a -c foo -b -d bar leftover1 leftover2
