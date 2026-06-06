#!/usr/bin/env zshrs
# Render a chess board from FEN position notation.

render_fen() {
    local fen=$1
    # Take only the placement portion (before first space).
    local placement=${fen%% *}
    local -a rows=( ${(s|/|)placement} )

    echo "  a b c d e f g h"
    local r=8
    for row in "${rows[@]}"; do
        printf "%d" $r
        local i
        for ((i=1; i<=${#row}; i++)); do
            local ch=${row[i]}
            case $ch in
                [1-8])
                    local n=$ch j
                    for ((j=0; j<n; j++)); do printf " ."; done
                    ;;
                *) printf " %s" "$ch" ;;
            esac
        done
        printf "  %d\n" $r
        (( r-- ))
    done
    echo "  a b c d e f g h"
}

echo "── starting position ──"
render_fen "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"

echo
echo "── after 1. e4 ──"
render_fen "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1"

echo
echo "── Italian opening (after a few moves) ──"
render_fen "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 4 4"

echo
echo "── empty board ──"
render_fen "8/8/8/8/8/8/8/8 w - - 0 1"

echo
echo "── endgame K+P vs K ──"
render_fen "8/8/8/4k3/8/8/4P3/4K3 w - - 0 1"

echo
echo "── highlight square ──"
mark_square() {
    local file=$1 rank=$2
    local files=(a b c d e f g h)
    local file_idx
    for ((i=1; i<=8; i++)); do
        [[ ${files[i]} == $file ]] && file_idx=$i
    done
    echo "  ${file}${rank} = column $file_idx row $rank"
}
mark_square e 4
mark_square a 1
mark_square h 8

# === ztest assertions ===
zassert_eq "$(mark_square e 4)" "  e4 = column 5 row 4" "e4 → col 5"
zassert_eq "$(mark_square a 1)" "  a1 = column 1 row 1" "a1 → col 1"
zassert_eq "$(mark_square h 8)" "  h8 = column 8 row 8" "h8 → col 8"
empty_out=$(render_fen "8/8/8/8/8/8/8/8 w - - 0 1")
zassert_contains "$empty_out" "  a b c d e f g h" "empty board has file header"
zassert_contains "$empty_out" "8 . . . . . . . .  8" "empty board rank 8"
zassert_contains "$empty_out" "1 . . . . . . . .  1" "empty board rank 1"
start_out=$(render_fen "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
zassert_contains "$start_out" "8 r n b q k b n r  8" "start: black back rank"
zassert_contains "$start_out" "1 R N B Q K B N R  1" "start: white back rank"
zassert_contains "$start_out" "7 p p p p p p p p  7" "start: black pawns"
ztest_run
