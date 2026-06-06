#!/usr/bin/env zshrs
# Minesweeper — generate board + count adjacent mines + reveal flood.

ROWS=8
COLS=12
MINES=15

typeset -a BOARD     # M or .
typeset -a COUNTS    # 0..8 adjacent
typeset -a REVEALED  # 0/1

cell_idx() {
    local r=$1 c=$2
    echo $(( (r-1) * COLS + c ))
}

is_valid() {
    local r=$1 c=$2
    (( r >= 1 && r <= ROWS && c >= 1 && c <= COLS ))
}

generate() {
    local mines=$1 i r c idx
    BOARD=()
    REVEALED=()
    for ((i=1; i<=ROWS*COLS; i++)); do
        BOARD[i]="."
        REVEALED[i]=0
    done
    # Place mines.
    local placed=0
    while (( placed < mines )); do
        r=$(( RANDOM % ROWS + 1 ))
        c=$(( RANDOM % COLS + 1 ))
        idx=$(cell_idx $r $c)
        if [[ ${BOARD[idx]} != "M" ]]; then
            BOARD[idx]="M"
            (( placed++ ))
        fi
    done
}

count_adjacent() {
    COUNTS=()
    local r c idx nr nc nidx n
    for ((r=1; r<=ROWS; r++)); do
        for ((c=1; c<=COLS; c++)); do
            idx=$(cell_idx $r $c)
            if [[ ${BOARD[idx]} == "M" ]]; then
                COUNTS[idx]="M"
                continue
            fi
            n=0
            for dr in -1 0 1; do
                for dc in -1 0 1; do
                    (( dr == 0 && dc == 0 )) && continue
                    nr=$((r + dr))
                    nc=$((c + dc))
                    if is_valid $nr $nc; then
                        nidx=$(cell_idx $nr $nc)
                        [[ ${BOARD[nidx]} == "M" ]] && (( n++ ))
                    fi
                done
            done
            COUNTS[idx]=$n
        done
    done
}

print_board() {
    local mode=$1   # full | hidden
    local r c idx
    printf "    "
    for ((c=1; c<=COLS; c++)); do printf "%2d" $c; done
    echo
    for ((r=1; r<=ROWS; r++)); do
        printf "  %2d " $r
        for ((c=1; c<=COLS; c++)); do
            idx=$(cell_idx $r $c)
            if [[ $mode == hidden ]] && (( ! REVEALED[idx] )); then
                printf " #"
            else
                local v=${COUNTS[idx]}
                if [[ $v == M ]]; then
                    printf " ✱"
                elif [[ $v == 0 ]]; then
                    printf " ·"
                else
                    printf " %d" $v
                fi
            fi
        done
        echo
    done
}

# Flood reveal: BFS from (r,c).
reveal_flood() {
    local r=$1 c=$2 idx
    typeset -a queue
    queue=("$r $c")
    local qi=1
    while (( qi <= ${#queue} )); do
        set -- ${=queue[qi]}
        (( qi++ ))
        local cr=$1 cc=$2
        idx=$(cell_idx $cr $cc)
        (( REVEALED[idx] )) && continue
        REVEALED[idx]=1
        [[ ${BOARD[idx]} == "M" ]] && continue
        if [[ ${COUNTS[idx]} == 0 ]]; then
            local dr dc nr nc
            for dr in -1 0 1; do
                for dc in -1 0 1; do
                    (( dr == 0 && dc == 0 )) && continue
                    nr=$((cr + dr))
                    nc=$((cc + dc))
                    if is_valid $nr $nc; then
                        queue+=("$nr $nc")
                    fi
                done
            done
        fi
    done
}

reveal_count() {
    local n=0 i
    for ((i=1; i<=ROWS*COLS; i++)); do
        (( REVEALED[i] )) && (( n++ ))
    done
    echo $n
}

mine_count() {
    local n=0 i
    for ((i=1; i<=ROWS*COLS; i++)); do
        [[ ${BOARD[i]} == "M" ]] && (( n++ ))
    done
    echo $n
}

RANDOM=42
generate $MINES
count_adjacent

echo "── full board (mines + counts) ──"
print_board full
echo "  mines: $(mine_count)"

echo
echo "── reveal click on (4,5) ──"
reveal_flood 4 5
echo "  revealed: $(reveal_count) / $(( ROWS * COLS ))"
print_board hidden

echo
echo "── reveal click on (1,1) ──"
reveal_flood 1 1
echo "  revealed: $(reveal_count)"
print_board hidden

echo
echo "── reveal click on (8,12) ──"
reveal_flood 8 12
echo "  revealed: $(reveal_count)"
print_board hidden

echo
echo "── mine density check ──"
total=$(( ROWS * COLS ))
echo "  board: ${ROWS}x${COLS} = $total cells"
echo "  mines: $MINES ($(( MINES * 100 / total ))%)"

echo
echo "── adjacency-count histogram ──"
typeset -A hist
for ((i=1; i<=ROWS*COLS; i++)); do
    [[ ${COUNTS[i]} == "M" ]] && continue
    (( hist[${COUNTS[i]}]++ ))
done
for k in 0 1 2 3 4 5 6 7 8; do
    c=${hist[$k]:-0}
    bar=""
    for ((b=0; b<c; b++)); do bar+="█"; done
    printf "  %d adj: %3d %s\n" $k $c "$bar"
done

# === ztest assertions ===
RANDOM=42
generate $MINES
count_adjacent
zassert_eq "$(mine_count)" "$MINES"           "mine_count matches placed"
zassert_eq "$(( ROWS * COLS ))" "96"          "board cells = ROWS*COLS"
zassert_eq "$(cell_idx 1 1)" "1"              "cell_idx (1,1)"
zassert_eq "$(cell_idx 8 12)" "96"            "cell_idx last"
is_valid 1 1 && r=1 || r=0
zassert_eq "$r" "1"                           "is_valid (1,1)"
is_valid 0 1 && r=1 || r=0
zassert_eq "$r" "0"                           "is_valid (0,1) rejected"
is_valid 9 1 && r=1 || r=0
zassert_eq "$r" "0"                           "is_valid (9,1) rejected"
# After reveal flood from (4,5), at least one cell revealed.
REVEALED=()
for ((i=1; i<=ROWS*COLS; i++)); do REVEALED[i]=0; done
reveal_flood 4 5
zassert_gt "$(reveal_count)" "0"              "reveal_flood revealed cells"
ztest_run
