#!/usr/bin/env zshrs
# Tiny text adventure — scripted path through rooms.

typeset -A ROOM_NAME ROOM_DESC ROOM_EXITS

setup() {
    ROOM_NAME[start]="Forest Edge"
    ROOM_DESC[start]="You stand at the edge of a dark forest. A narrow path leads north."
    ROOM_EXITS[start]="n=forest"

    ROOM_NAME[forest]="Dark Forest"
    ROOM_DESC[forest]="Trees press in close. You hear water to the east, a cave to the west."
    ROOM_EXITS[forest]="s=start,e=river,w=cave"

    ROOM_NAME[river]="River Bank"
    ROOM_DESC[river]="A clear river flows past. A boat is tied to a post."
    ROOM_EXITS[river]="w=forest,n=island"

    ROOM_NAME[cave]="Dark Cave"
    ROOM_DESC[cave]="The cave smells of damp earth. Something glints in the darkness."
    ROOM_EXITS[cave]="e=forest,treasure=loot"

    ROOM_NAME[island]="Small Island"
    ROOM_DESC[island]="A tiny island with a single tree. You see footprints leading nowhere."
    ROOM_EXITS[island]="s=river"

    ROOM_NAME[loot]="Treasure Pile"
    ROOM_DESC[loot]="Gold! Jewels! A glowing artifact!"
    ROOM_EXITS[loot]="back=cave"
}

CURRENT=start

look() {
    local r=$CURRENT
    echo
    echo "── ${ROOM_NAME[$r]} ──"
    echo "${ROOM_DESC[$r]}"
    echo "Exits:"
    local exits=${ROOM_EXITS[$r]}
    for kv in ${(s/,/)exits}; do
        local k=${kv%%=*}
        local dest=${kv#*=}
        echo "  $k → ${ROOM_NAME[$dest]:-?}"
    done
}

move() {
    local dir=$1
    local exits=${ROOM_EXITS[$CURRENT]}
    for kv in ${(s/,/)exits}; do
        local k=${kv%%=*}
        local dest=${kv#*=}
        if [[ $k == $dir ]]; then
            CURRENT=$dest
            return 0
        fi
    done
    echo "  cannot go $dir from here"
    return 1
}

setup

echo "── intro ──"
look

echo
echo "── go north ──"
move n
look

echo
echo "── go east ──"
move e
look

echo
echo "── go north (island) ──"
move n
look

echo
echo "── try invalid dir ──"
move down

echo
echo "── return south, west, then treasure ──"
move s
move w
look
move treasure
look

echo
echo "── final inventory ──"
echo "VICTORY! you reached the treasure pile."
echo
echo "── visited rooms ──"
echo "  start → forest → river → island → river → forest → cave → loot"

# === ztest assertions ===
zassert_eq "${ROOM_NAME[start]}"   "Forest Edge"  "start room name"
zassert_eq "${ROOM_NAME[forest]}"  "Dark Forest"  "forest room name"
zassert_eq "${ROOM_NAME[loot]}"    "Treasure Pile" "loot room name"
zassert_contains "${ROOM_EXITS[start]}" "n=forest" "start has north exit"
zassert_contains "${ROOM_EXITS[forest]}" "e=river" "forest east → river"
# test move
CURRENT=start
move n
zassert_eq "$CURRENT" "forest"   "move n from start → forest"
move e
zassert_eq "$CURRENT" "river"    "move e from forest → river"
# invalid move
CURRENT=start
move down 2>&1 | grep -q "cannot go" && zassert_ok 1 "invalid move rejected" || zassert_ok 0 "invalid move should be rejected"
ztest_run
