#!/usr/bin/env zshrs
# Advanced associative-array iteration patterns.

typeset -A m
m[apple]=10
m[banana]=5
m[cherry]=20
m[date]=8
m[elderberry]=15
m[fig]=3
m[grape]=12

echo "── default iteration (random order in zsh) ──"
for k in ${(k)m}; do echo "  $k → ${m[$k]}"; done

echo "── sorted by key ──"
for k in ${(ko)m}; do echo "  $k → ${m[$k]}"; done

echo "── reverse-sorted by key ──"
for k in ${(kO)m}; do echo "  $k → ${m[$k]}"; done

echo "── (@kv) k/v pairs ──"
for k v in "${(@kv)m}"; do
    printf "  %-12s = %d\n" "$k" $v
done | sort

echo "── filter by value ──"
echo "  values > 10:"
for k in ${(ko)m}; do
    (( m[$k] > 10 )) && echo "    $k → ${m[$k]}"
done

echo "── filter by key pattern ──"
echo "  keys starting with [a-d]:"
for k in ${(ko)m}; do
    [[ $k == [a-d]* ]] && echo "    $k"
done

echo "── sort by value (ascending) ──"
pairs=()
for k v in "${(@kv)m}"; do
    pairs+=("$(printf '%05d %s' $v $k)")
done
for p in ${(o)pairs}; do
    set -- $=p
    printf "  %-12s = %d\n" $2 $((10#$1))
done

echo "── top 3 by value ──"
for p in ${(O)pairs}; do
    set -- $=p
    printf "  %-12s = %d\n" $2 $((10#$1))
done | head -3

echo "── transform values (double each) ──"
typeset -A doubled
for k v in "${(@kv)m}"; do
    doubled[$k]=$(( v * 2 ))
done
echo "  doubled:"
for k in ${(ko)doubled}; do
    echo "    $k: ${m[$k]} → ${doubled[$k]}"
done

echo "── partition into two ──"
typeset -A small large
for k v in "${(@kv)m}"; do
    if (( v < 10 )); then
        small[$k]=$v
    else
        large[$k]=$v
    fi
done
echo "  small (<10): ${(ko)small}"
echo "  large (≥10): ${(ko)large}"

echo "── merge two assocs ──"
typeset -A merged
for k v in "${(@kv)small}"; do merged[$k]=$v; done
for k v in "${(@kv)large}"; do merged[$k]=$v; done
echo "  merged size: ${#merged[@]}"

echo "── stats ──"
sum=0; count=${#m[@]}
for v in ${(v)m}; do (( sum += v )); done
echo "  count: $count, sum: $sum, mean: $(( sum / count ))"

min_v=999; max_v=0
for v in ${(v)m}; do
    (( v < min_v )) && min_v=$v
    (( v > max_v )) && max_v=$v
done
echo "  min: $min_v, max: $max_v, range: $((max_v - min_v))"

# === ztest assertions ===
zassert_eq "${#m[@]}" 7              "7 entries in m"
zassert_eq "$count" 7                "count matches size"
zassert_eq "$sum" 73                 "sum 10+5+20+8+15+3+12"
zassert_eq "$(( sum / count ))" 10   "mean (integer)"
zassert_eq "$min_v" 3                "min value (fig=3)"
zassert_eq "$max_v" 20               "max value (cherry=20)"
zassert_eq "${doubled[apple]}" 20    "doubled[apple]=20"
zassert_eq "${doubled[fig]}" 6       "doubled[fig]=6"
zassert_eq "${#small[@]}" 3          "3 small entries (<10)"
zassert_eq "${#large[@]}" 4          "4 large entries (>=10)"
zassert_eq "${#merged[@]}" 7         "merged keeps all 7 keys"
ztest_run
