#!/usr/bin/env zshrs
# Trie (prefix tree) — insert/search/prefix-count/autocomplete.

typeset -A CHILDREN END_OF_WORD
typeset -gi NEXT=0
ROOT=0

trie_init() {
    CHILDREN=()
    END_OF_WORD=()
    NEXT=0
    ROOT=0
    NEXT=1
    ROOT=1
    END_OF_WORD[1]=0
}

trie_insert() {
    local word=$1 cur=$ROOT i ch key
    for ((i=1; i<=${#word}; i++)); do
        ch=${word[i]}
        key="${cur}_${ch}"
        local nxt="${CHILDREN[$key]}"
        if [[ -z $nxt ]]; then
            (( NEXT++ ))
            CHILDREN[$key]=$NEXT
            END_OF_WORD[$NEXT]=0
            cur=$NEXT
        else
            cur=$nxt
        fi
    done
    END_OF_WORD[$cur]=1
}

# Returns 0 if word in trie, 1 if just a prefix, 2 if not.
trie_search() {
    local word=$1 cur=$ROOT i ch key nxt
    for ((i=1; i<=${#word}; i++)); do
        ch=${word[i]}
        key="${cur}_${ch}"
        nxt="${CHILDREN[$key]}"
        [[ -z $nxt ]] && return 2
        cur=$nxt
    done
    if (( END_OF_WORD[$cur] == 1 )); then return 0; else return 1; fi
}

# DFS from given node, collect all words via a prefix.
collect_words() {
    local cur=$1 prefix=$2
    typeset -ga WORDS_FOUND
    # If first call from outside, reset.
    if [[ -z $3 ]]; then WORDS_FOUND=(); fi
    if (( END_OF_WORD[$cur] == 1 )); then
        WORDS_FOUND+=("$prefix")
    fi
    # Iterate children of cur.
    local key ch nxt
    for key in "${(@k)CHILDREN}"; do
        if [[ $key == ${cur}_* ]]; then
            ch="${key#${cur}_}"
            nxt="${CHILDREN[$key]}"
            collect_words $nxt "${prefix}${ch}" inner
        fi
    done
}

trie_autocomplete() {
    local prefix=$1 cur=$ROOT i ch key nxt
    for ((i=1; i<=${#prefix}; i++)); do
        ch=${prefix[i]}
        key="${cur}_${ch}"
        nxt="${CHILDREN[$key]}"
        if [[ -z $nxt ]]; then
            typeset -ga WORDS_FOUND
            WORDS_FOUND=()
            return
        fi
        cur=$nxt
    done
    collect_words $cur "$prefix"
}

# Count words starting with prefix.
trie_count_prefix() {
    trie_autocomplete "$1"
    echo ${#WORDS_FOUND}
}

# Word counts: pop, popular, popularly, popsicle
echo "── insert dictionary ──"
trie_init
words=(
    pop popular popularly popsicle popcorn pope
    app apple application applied apply
    bat batch batter battery
    car card cards care careful careless
    do dog dogs doing done
    ze zen zebra zero
    code coder codes coding
)
for w in "${words[@]}"; do
    trie_insert "$w"
done
echo "  inserted ${#words} words"
echo "  unique nodes: $NEXT"

echo
echo "── search ──"
queries=(pop popular popularly popcorn pop popmania batch xyz cat cards)
for q in "${queries[@]}"; do
    trie_search "$q"
    case $? in
        0) echo "  '$q' → ✓ word" ;;
        1) echo "  '$q' → · prefix only" ;;
        2) echo "  '$q' → ✗ not in trie" ;;
    esac
done

echo
echo "── autocomplete ──"
prefixes=(pop app car do ze code z xy)
for p in "${prefixes[@]}"; do
    trie_autocomplete "$p"
    if (( ${#WORDS_FOUND} > 0 )); then
        echo "  '$p' → ${WORDS_FOUND[*]}"
    else
        echo "  '$p' → (no completions)"
    fi
done

echo
echo "── count words with prefix ──"
for p in pop app car do z ze code "" a; do
    c=$(trie_count_prefix "$p")
    printf "  '%-6s' → %d\n" "$p" $c
done

echo
echo "── longest common prefix ──"
common_words=(applemartini appletree applefritter applecart)
trie_init
for w in "${common_words[@]}"; do
    trie_insert "$w"
done

# Walk root until branching.
cur=$ROOT
prefix=""
while true; do
    # Count children of cur.
    local nchildren=0 only_child=""
    for key in "${(@k)CHILDREN}"; do
        if [[ $key == ${cur}_* ]]; then
            (( nchildren++ ))
            only_child="${key#${cur}_}"
        fi
    done
    if (( nchildren != 1 )); then break; fi
    if (( END_OF_WORD[$cur] == 1 )); then break; fi
    prefix+="$only_child"
    cur=${CHILDREN[${cur}_${only_child}]}
done
echo "  words: ${common_words[*]}"
echo "  longest common prefix: '$prefix'"

echo
echo "── stats ──"
trie_init
for w in "${words[@]}"; do
    trie_insert "$w"
done
echo "  total words: ${#words}"
echo "  trie nodes:  $NEXT"
echo "  edges:       ${#CHILDREN}"
total_chars=0
for w in "${words[@]}"; do
    (( total_chars += ${#w} ))
done
echo "  total chars (if flat): $total_chars"
echo "  ratio of nodes/chars: $(( NEXT * 100 / total_chars ))%"
echo "  (lower ratio = better compression via shared prefixes)"

# === ztest assertions ===
trie_init
trie_insert apple
trie_insert app
trie_insert apt
# trie_search status codes
if trie_search apple; then zassert_ok 1 "apple found"
else zassert_ok 0 "apple should find"; fi
trie_search app
zassert_eq $? 0 "app is word"
trie_search appl
zassert_eq $? 1 "appl is prefix only"
trie_search xyz
zassert_eq $? 2 "xyz not in trie"
# Cardinality
zassert_ok "$NEXT"     "node count > 0"
zassert_ok "${#CHILDREN}" "children edges > 0"
# Total words inserted in earlier section
zassert_eq "${#words}" 34 "34 dict words"
ztest_run
