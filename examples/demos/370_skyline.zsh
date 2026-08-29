#!/usr/bin/env zshrs
# Skyline problem — given N buildings as (left,right,height) triples,
# produce the outline as a sequence of (x,height) key-points.
#
# Classic divide-and-conquer formulation (Manber 1989):
#   skyline(B) where |B|=1 → [(L,H),(R,0)]
#   skyline(B) where |B|>1 → merge(skyline(left half), skyline(right half))
#
# The merge step walks both skylines in x order, tracking current
# heights from each side, emitting a point when the max-of-the-two
# changes.

# Buildings stored flat:
#   BL[i] BR[i] BH[i]  for i in 1..N
typeset -ga BL BR BH

add_building() {
    BL+=("$1")
    BR+=("$2")
    BH+=("$3")
}

# Compute skyline of building range [lo..hi]; returns flat "x1 h1 x2 h2 ..."
skyline() {
    local lo=$1 hi=$2
    if (( lo == hi )); then
        echo "${BL[lo]} ${BH[lo]} ${BR[lo]} 0"
        return
    fi
    local mid=$(( (lo + hi) / 2 ))
    local left=$(skyline $lo $mid)
    local right=$(skyline $(( mid + 1 )) $hi)
    merge_skylines "$left" "$right"
}

# Merge two flat skylines (each x,h pairs space-separated).
merge_skylines() {
    local -a A B
    A=(${(z)1})
    B=(${(z)2})
    local i=1 j=1 h1=0 h2=0
    local -a OUT
    local last_h=0
    local x   # NB: declare once — a bare `local x` re-declaration inside the
              # loop echoes "x=N" onto stdout and corrupts the merged skyline.
    while (( i <= ${#A} && j <= ${#B} )); do
        if (( A[i] < B[j] )); then
            x=${A[i]}
            h1=${A[i+1]}
            (( i += 2 ))
        elif (( A[i] > B[j] )); then
            x=${B[j]}
            h2=${B[j+1]}
            (( j += 2 ))
        else
            x=${A[i]}
            h1=${A[i+1]}
            h2=${B[j+1]}
            (( i += 2 ))
            (( j += 2 ))
        fi
        local max_h=$h1
        (( h2 > max_h )) && max_h=$h2
        if (( max_h != last_h )); then
            OUT+=("$x" "$max_h")
            last_h=$max_h
        fi
    done
    while (( i <= ${#A} )); do
        if (( A[i+1] != last_h )); then
            OUT+=("${A[i]}" "${A[i+1]}")
            last_h=${A[i+1]}
        fi
        (( i += 2 ))
    done
    while (( j <= ${#B} )); do
        if (( B[j+1] != last_h )); then
            OUT+=("${B[j]}" "${B[j+1]}")
            last_h=${B[j+1]}
        fi
        (( j += 2 ))
    done
    echo "${OUT[@]}"
}

# ASCII render — for visual sanity.
render() {
    local -a pts
    pts=(${(z)1})
    local maxh=0 i
    for ((i=2; i<=${#pts}; i+=2)); do
        (( pts[i] > maxh )) && maxh=${pts[i]}
    done
    local maxx=0
    for ((i=1; i<=${#pts}; i+=2)); do
        (( pts[i] > maxx )) && maxx=${pts[i]}
    done
    # Build height map indexed by x ∈ [0..maxx].
    local -a hmap
    local cur=0
    for ((i=0; i<=maxx; i++)); do hmap+=(0); done
    local pi=1
    for ((i=0; i<=maxx; i++)); do
        while (( pi <= ${#pts} && pts[pi] == i )); do
            cur=${pts[pi+1]}
            (( pi += 2 ))
        done
        hmap[i+1]=$cur
    done
    local row col
    for ((row=maxh; row>=1; row--)); do
        local line=""
        for ((col=1; col<=maxx+1; col++)); do
            if (( hmap[col] >= row )); then
                line+="█"
            else
                line+=" "
            fi
        done
        echo "$line"
    done
}

# === Test fixture 1 ===
BL=(); BR=(); BH=()
add_building 1 4 8
add_building 3 7 12
add_building 6 9 10
add_building 12 14 5
echo "=== buildings 1: (1,4,8) (3,7,12) (6,9,10) (12,14,5) ==="
sk1=$(skyline 1 ${#BL})
echo "skyline: $sk1"
echo
render "$sk1"

# === Test fixture 2 ===
BL=(); BR=(); BH=()
add_building 2 9 10
add_building 3 7 15
add_building 5 12 12
add_building 15 20 10
add_building 19 24 8
echo
echo "=== buildings 2: (2,9,10) (3,7,15) (5,12,12) (15,20,10) (19,24,8) ==="
sk2=$(skyline 1 ${#BL})
echo "skyline: $sk2"

# === ztest ===
zassert_eq "$sk1" "1 8 3 12 7 10 9 0 12 5 14 0" "skyline 1 outline"
zassert_eq "$sk2" "2 10 3 15 7 12 12 0 15 10 20 8 24 0" "skyline 2 outline"

# Single building → degenerate skyline.
BL=(); BR=(); BH=()
add_building 5 10 7
zassert_eq "$(skyline 1 1)" "5 7 10 0" "single-building skyline"

# Two non-overlapping → middle drops to 0.
BL=(); BR=(); BH=()
add_building 1 3 5
add_building 5 7 8
zassert_eq "$(skyline 1 2)" "1 5 3 0 5 8 7 0" "two non-overlapping"

ztest_run
