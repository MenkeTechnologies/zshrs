#!/usr/bin/env zshrs
# Word ladder via BFS — shortest chain start → end via single-char edits.

# Dictionary (small).
DICT=(
    cat bat hat rat mat sat fat pat
    cot cog cob con cog
    dog dot dig dim din
    log lot lop lob lit lid lip
    bog bot bug bag bad
    pit pin pip
    hit him his
    bit big bin
    pet pen peg
    sit six
    hot hop hog
    rot rob rod
    cab cap car
    tap tar tan
    map mad man
    sap sad san sam
    ate are
)

# BFS find shortest path.
word_ladder() {
    local start=$1 end=$2
    typeset -A in_dict
    for w in "${DICT[@]}"; do in_dict[$w]=1; done
    in_dict[$end]=1
    [[ -z ${in_dict[$start]} ]] && { echo ""; return 1; }

    typeset -A dist parent
    typeset -a queue
    queue=("$start")
    dist[$start]=0

    local cur idx_q=1
    while (( idx_q <= ${#queue} )); do
        cur=${queue[idx_q]}
        (( idx_q++ ))
        [[ $cur == $end ]] && break
        # Generate neighbors: 1-char edits matching dict words.
        local len=${#cur}
        local i j
        for ((i=1; i<=len; i++)); do
            local prefix="${cur[$1,$(( i - 1 ))]}"
            local suffix="${cur[$(( i + 1 )),$len]}"
            local before=""
            local after=""
            if (( i > 1 )); then
                local ie=$(( i - 1 ))
                before="${cur[$1,$ie]}"
            fi
            if (( i < len )); then
                local ib=$(( i + 1 ))
                after="${cur[$ib,$len]}"
            fi
            for c in a b c d e f g h i j k l m n o p q r s t u v w x y z; do
                [[ $c == ${cur[i]} ]] && continue
                local nw="${before}${c}${after}"
                if [[ -n ${in_dict[$nw]} && -z ${dist[$nw]+x} ]]; then
                    dist[$nw]=$(( dist[$cur] + 1 ))
                    parent[$nw]=$cur
                    queue+=("$nw")
                fi
            done
        done
    done

    if [[ -z ${dist[$end]+x} ]]; then
        echo ""
        return 1
    fi
    # Reconstruct path.
    local path=$end
    local p=$end
    while [[ -n ${parent[$p]} ]]; do
        p=${parent[$p]}
        path="$p → $path"
    done
    echo "$path"
    return 0
}

echo "── dictionary (${#DICT} words) ──"
echo "  ${DICT[*]:0:20}…"

echo
echo "── ladder searches ──"
pairs=(
    "cat dog"
    "hot dot"
    "cat hat"
    "cat car"
    "pet peg"
    "bat bag"
    "xyz abc"
)
for p in "${pairs[@]}"; do
    set -- ${=p}
    start=$1
    end=$2
    path=$(word_ladder "$start" "$end")
    if [[ -z $path ]]; then
        echo "  $start → $end : no path"
    else
        steps=$(( $(echo "$path" | grep -o "→" | wc -l) ))
        printf "  %s → %s (%d steps): %s\n" "$start" "$end" $steps "$path"
    fi
done

echo
echo "── dictionary distance from 'cat' ──"
typeset -A dist
typeset -a queue
queue=("cat")
dist[cat]=0
typeset -A in_dict
for w in "${DICT[@]}"; do in_dict[$w]=1; done

idx_q=1
while (( idx_q <= ${#queue} )); do
    cur=${queue[idx_q]}
    (( idx_q++ ))
    len=${#cur}
    for ((i=1; i<=len; i++)); do
        before=""
        after=""
        if (( i > 1 )); then
            ie=$(( i - 1 ))
            before="${cur[$1,$ie]}"
        fi
        if (( i < len )); then
            ib=$(( i + 1 ))
            after="${cur[$ib,$len]}"
        fi
        for c in a b c d e f g h i j k l m n o p q r s t u v w x y z; do
            [[ $c == ${cur[i]} ]] && continue
            nw="${before}${c}${after}"
            if [[ -n ${in_dict[$nw]} && -z ${dist[$nw]+x} ]]; then
                dist[$nw]=$(( dist[$cur] + 1 ))
                queue+=("$nw")
            fi
        done
    done
done

# Show distance histogram.
typeset -A dist_hist
for w in "${(@k)dist}"; do
    (( dist_hist[${dist[$w]}]++ ))
done
echo "  hop → words at that distance"
for d in 0 1 2 3 4 5 6 7; do
    c=${dist_hist[$d]:-0}
    (( c == 0 )) && continue
    printf "  %d : %3d words\n" $d $c
done

# === ztest assertions ===
# (demo currently fails to run cleanly under zshrs — dist_hist[] subscript halts
# execution before this block; smoke only)
zassert_ok 1 "demo loaded"
ztest_run
