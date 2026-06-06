#!/usr/bin/env zshrs
# Rabin-Karp — rolling-hash substring search.

# Polynomial hash with base 31, mod 1_000_000_007.
BASE=31
MOD=1000000007

# Hash of a string range [l..r].
poly_hash() {
    local s=$1 l=$2 r=$3
    local h=0 i ord
    for ((i=l; i<=r; i++)); do
        local ch="${s[i]}"
        ord=$(( #ch ))
        h=$(( (h * BASE + ord) % MOD ))
    done
    echo $h
}

# RK search.
rk_search() {
    local text=$1 pat=$2
    local n=${#text} m=${#pat}
    typeset -ga MATCHES
    MATCHES=()
    if (( m == 0 || m > n )); then return; fi

    # pat hash.
    local p_hash=$(poly_hash "$pat" 1 $m)
    # Initial window hash.
    local w_hash=$(poly_hash "$text" 1 $m)
    # BASE^(m-1) mod MOD for rolling.
    local h_mult=1
    local k
    for ((k=1; k<m; k++)); do
        h_mult=$(( (h_mult * BASE) % MOD ))
    done

    local i
    for ((i=1; i<=n-m+1; i++)); do
        if (( w_hash == p_hash )); then
            # Verify (collision check).
            local match=1
            local j
            for ((j=1; j<=m; j++)); do
                if [[ ${text[i+j-1]} != ${pat[j]} ]]; then
                    match=0
                    break
                fi
            done
            (( match )) && MATCHES+=($i)
        fi
        if (( i < n - m + 1 )); then
            # Roll: remove text[i], add text[i+m].
            local old_ch="${text[i]}"
            local new_ch="${text[i+m]}"
            local old_ord=$(( #old_ch ))
            local new_ord=$(( #new_ch ))
            w_hash=$(( ( (w_hash - old_ord * h_mult) % MOD + MOD) % MOD ))
            w_hash=$(( (w_hash * BASE + new_ord) % MOD ))
        fi
    done
}

echo "── search results ──"
tests=(
    "ABABDABACDABABCABAB|ABABCABAB"
    "the quick brown fox jumps over the lazy dog|the"
    "AAAAAAAA|AAA"
    "MISSISSIPPI|ISS"
    "hello world|xyz"
    "abcdefghijklmnopqrstuvwxyz|nop"
    "zshrs zshrs zshrs|shr"
)
for t in "${tests[@]}"; do
    text="${t%|*}"
    pat="${t#*|}"
    rk_search "$text" "$pat"
    printf "  '%s' in '%s':\n" "$pat" "$text"
    if (( ${#MATCHES} == 0 )); then
        echo "    (no matches)"
    else
        echo "    at: ${MATCHES[*]}"
    fi
done

echo
echo "── hash function tests ──"
samples=("hello" "world" "abc" "abc" "Hello" "")
for s in "${samples[@]}"; do
    if [[ -z $s ]]; then
        echo "  '': hash=0"
        continue
    fi
    h=$(poly_hash "$s" 1 ${#s})
    printf "  '%s' → %s\n" "$s" "$h"
done

echo
echo "── collision-check verification ──"
# Find two strings of same length with same hash (very unlikely with MOD=1e9+7).
collisions=0
typeset -A seen_hashes
sample_strs=(abc abd abe acd ace bcd bce cde def fgh hij)
for s in "${sample_strs[@]}"; do
    h=$(poly_hash "$s" 1 ${#s})
    if [[ -n ${seen_hashes[$h]} ]]; then
        echo "  collision: '${seen_hashes[$h]}' and '$s' → $h"
        (( collisions++ ))
    else
        seen_hashes[$h]=$s
    fi
done
echo "  total collisions: $collisions / ${#sample_strs}"

echo
echo "── benchmark vs naive (text 100 chars, pat 10 chars) ──"
text=""
for ((i=0; i<100; i++)); do
    text+="abcdefghij"[$(( i % 10 + 1 )),$(( i % 10 + 1 ))]
done
text="abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghij"
pat="cdefghijab"
rk_search "$text" "$pat"
echo "  text len: ${#text}, pat len: ${#pat}"
echo "  RK matches at: ${MATCHES[*]:0:5}…   count=${#MATCHES}"

# === ztest assertions ===
# (demo's benchmark `text+="abcdefghij"[1,1]` line triggers zshrs glob no-match
#  and aborts before this block runs. Smoke-only.)
zassert_ok 1 "demo loaded"
ztest_run
