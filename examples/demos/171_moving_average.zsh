#!/usr/bin/env zshrs
# Moving / rolling statistics — mean, min, max, sliding window.

values=(7 4 9 11 5 8 3 12 6 10 14 2 7 9 11 5)

mean() {
    local sum=0 v n=$#
    for v in "$@"; do (( sum += v )); done
    echo $(( sum / n ))
}

min() {
    local m=$1
    shift
    local v
    for v in "$@"; do (( v < m )) && m=$v; done
    echo $m
}

max() {
    local m=$1
    shift
    local v
    for v in "$@"; do (( v > m )) && m=$v; done
    echo $m
}

echo "── data ──"
echo "${values[@]}"
echo "n=${#values[@]}, sum=$(IFS=+; echo $((${values[*]}))), mean=$(mean "${values[@]}")"
echo "min=$(min "${values[@]}"), max=$(max "${values[@]}")"

echo "── 3-window moving average ──"
n=${#values[@]}
for ((i=1; i<=n-2; i++)); do
    window=("${values[i]}" "${values[i+1]}" "${values[i+2]}")
    avg=$(mean "${window[@]}")
    printf "  i=%2d window=[%s] avg=%d\n" $i "${window[*]}" $avg
done

echo "── 5-window moving min/max/avg ──"
for ((i=1; i<=n-4; i++)); do
    w=("${values[i]}" "${values[i+1]}" "${values[i+2]}" "${values[i+3]}" "${values[i+4]}")
    printf "  i=%2d window=[%-13s] min=%d max=%d avg=%d\n" $i "${w[*]}" \
        $(min "${w[@]}") $(max "${w[@]}") $(mean "${w[@]}")
done

echo "── cumulative sum ──"
sum=0
for ((i=1; i<=n; i++)); do
    (( sum += values[i] ))
    printf "  i=%2d v=%2d cum=%d\n" $i ${values[i]} $sum
done

echo "── peak detection ──"
for ((i=2; i<n; i++)); do
    if (( values[i] > values[i-1] && values[i] > values[i+1] )); then
        printf "  peak at i=%d v=%d\n" $i ${values[i]}
    fi
done

echo "── trend (last 3 strictly increasing?) ──"
for ((i=1; i<=n-2; i++)); do
    if (( values[i] < values[i+1] && values[i+1] < values[i+2] )); then
        printf "  trend ↗ at i=%d (%d→%d→%d)\n" $i ${values[i]} ${values[i+1]} ${values[i+2]}
    fi
done
