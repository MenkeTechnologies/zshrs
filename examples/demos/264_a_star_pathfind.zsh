#!/usr/bin/env zshrs
# A* pathfinding on a small ASCII grid.

# Grid: '.' walkable, '#' wall, 'S' start, 'G' goal.
typeset -a GRID
GRID=(
    "S....#....."
    ".##..#..##."
    ".#...#..#.."
    ".#.###..#.."
    ".#......#.."
    ".########.."
    "..........."
    "....##....."
    "....#......"
    ".......G..."
)
ROWS=${#GRID}
COLS=11

cell() {
    local r=$1 c=$2 row
    (( r < 1 || r > ROWS || c < 1 || c > COLS )) && { echo "#"; return; }
    row=${GRID[r]}
    echo "${row[c]}"
}

manhattan() { echo $(( $1 < $3 ? $3-$1 : $1-$3 + ($2 < $4 ? $4-$2 : $2-$4) )); }

# Find S and G.
sr=0; sc=0; gr=0; gc=0
for ((r=1; r<=ROWS; r++)); do
    row=${GRID[r]}
    for ((c=1; c<=COLS; c++)); do
        ch=${row[c]}
        [[ $ch == S ]] && { sr=$r; sc=$c; }
        [[ $ch == G ]] && { gr=$r; gc=$c; }
    done
done

echo "── grid ──"
for ((r=1; r<=ROWS; r++)); do echo "  ${GRID[r]}"; done
echo "  start: ($sr,$sc)   goal: ($gr,$gc)"

# A* search.
typeset -A gscore fscore came_from
typeset -a open_set
start_key="${sr},${sc}"
goal_key="${gr},${gc}"
open_set=( "$start_key" )
gscore[$start_key]=0
fscore[$start_key]=$(( (sr-gr < 0 ? gr-sr : sr-gr) + (sc-gc < 0 ? gc-sc : sc-gc) ))

iters=0
found=0
while (( ${#open_set} > 0 )); do
    (( iters++ ))
    # Find min f in open_set.
    best=""
    best_f=999999
    for k in "${open_set[@]}"; do
        f=${fscore[$k]:-999999}
        if (( f < best_f )); then
            best=$k
            best_f=$f
        fi
    done
    [[ -z $best ]] && break
    if [[ $best == $goal_key ]]; then
        found=1
        break
    fi
    # Remove best from open_set.
    new_open=()
    for k in "${open_set[@]}"; do
        [[ $k == $best ]] && continue
        new_open+=("$k")
    done
    open_set=("${new_open[@]}")
    # Expand neighbors.
    cur_r=${best%,*}
    cur_c=${best#*,}
    for d in "0 1" "0 -1" "1 0" "-1 0"; do
        set -- ${=d}
        nr=$(( cur_r + $1 ))
        nc=$(( cur_c + $2 ))
        (( nr < 1 || nr > ROWS || nc < 1 || nc > COLS )) && continue
        ch=$(cell $nr $nc)
        [[ $ch == "#" ]] && continue
        nkey="${nr},${nc}"
        tentative=$(( gscore[$best] + 1 ))
        cur_g=${gscore[$nkey]:-999999}
        if (( tentative < cur_g )); then
            came_from[$nkey]=$best
            gscore[$nkey]=$tentative
            # f = g + h
            dr=$(( nr < gr ? gr-nr : nr-gr ))
            dc=$(( nc < gc ? gc-nc : nc-gc ))
            fscore[$nkey]=$(( tentative + dr + dc ))
            in_open=0
            for k in "${open_set[@]}"; do
                [[ $k == $nkey ]] && in_open=1
            done
            (( ! in_open )) && open_set+=("$nkey")
        fi
    done
done

echo
echo "── A* result ──"
echo "  iterations: $iters"
if (( found )); then
    # Reconstruct path.
    path=( "$goal_key" )
    cur=$goal_key
    while [[ -n ${came_from[$cur]} ]]; do
        cur=${came_from[$cur]}
        path=("$cur" "${path[@]}")
    done
    echo "  path length: $(( ${#path} - 1 )) steps"
    echo "  path: ${path[1]} → ${path[-1]} via ${#path} cells"
    # Mark path on grid.
    declare -a marked
    for ((r=1; r<=ROWS; r++)); do marked+=("${GRID[r]}"); done
    for p in "${path[@]}"; do
        pr=${p%,*}
        pc=${p#*,}
        row=${marked[pr]}
        ch=${row[pc]}
        [[ $ch == S || $ch == G ]] && continue
        marked[pr]="${row[1,pc-1]}*${row[pc+1,${#row}]}"
    done
    echo
    echo "  marked path:"
    for ((r=1; r<=ROWS; r++)); do echo "    ${marked[r]}"; done
else
    echo "  no path found"
fi

# === ztest assertions ===
zassert_eq "$ROWS"            10        "grid rows"
zassert_eq "$COLS"            11        "grid cols"
zassert_eq "$sr,$sc"          "1,1"     "start coord"
zassert_eq "$gr,$gc"          "10,8"    "goal coord"
zassert_eq "$found"           1         "A* found path"
zassert_eq "$(( ${#path} - 1 ))" 16     "path length steps"
zassert_eq "${path[1]}"       "1,1"     "path begins at start"
zassert_eq "${path[-1]}"      "10,8"    "path ends at goal"
zassert_eq "$(cell 1 1)"      "S"       "cell(1,1) is start"
zassert_eq "$(cell 10 8)"     "G"       "cell(10,8) is goal"
zassert_eq "$(cell 1 6)"      "#"       "cell(1,6) is wall"
ztest_run
