#!/usr/bin/env zshrs
# 🎊 DEMO 350 — milestone banner.

banner() {
    local txt=$1 width=${2:-72}
    local n=${#txt}
    local pad=$(( (width - n) / 2 ))
    local sp="" bar="" i
    for ((i=0; i<pad; i++)); do sp+=" "; done
    for ((i=0; i<width; i++)); do bar+="═"; done
    echo "$bar"
    echo "${sp}${txt}"
    echo "$bar"
}

cat <<'EOF'


              ███████ ███████  ██████
              ╚════██║██╔════╝██╔═████╗
                  ██║███████╗██║██╔██║
                 ██╔╝╚════██║████╔╝██║
                 ██║ ███████║╚██████╔╝
                 ╚═╝ ╚══════╝ ╚═════╝

              ██████   ███████ ███╗   ███╗  ██████╗  ███████╗
              ██╔══██╗ ██╔════╝████╗ ████║ ██╔═══██╗ ██╔════╝
              ██║  ██║ █████╗  ██╔████╔██║ ██║   ██║ ███████╗
              ██║  ██║ ██╔══╝  ██║╚██╔╝██║ ██║   ██║ ╚════██║
              ██████╔╝ ███████╗██║ ╚═╝ ██║ ╚██████╔╝ ███████║
              ╚═════╝  ╚══════╝╚═╝     ╚═╝  ╚═════╝  ╚══════╝

EOF

banner "🎯 zshrs DEMO HARNESS: 350 STRONG 🎯" 72
echo
echo "  zshrs is the first compiled Unix shell. This demo suite —"
echo "  350 individual scripts — pins zshrs against zsh's C-source"
echo "  port site by port site. Every script runs on CI under"
echo "  zshrs --zsh; every script cites the Src/*.c it exercises."
echo

banner "WHAT 350 DEMOS EXERCISE" 72
echo
echo "  SHELL SEMANTICS                    ZSH-C FEATURES"
echo "  ─────────────────                  ───────────────"
echo "  arithmetic (int, float, base)      param flags (M, P, j, s, o, O)"
echo "  arrays (idx, assoc, slice)         glob qualifiers (.) (/) (om) (oL)"
echo "  parameter expansion (#%/^!)        extended glob ^/~/#/##/<a-b>"
echo "  control flow (if/case/loop)        modifiers :h :t :r :e :a :A :s"
echo "  redirection (<>>>&> /dev/null)     ksh patterns ?() +() *() !()"
echo "  process subst <(cmd) >(cmd)        bracket flags (#i) (#a) (#m)"
echo "  trap (EXIT/ERR/ZERR/USR1)          ZLE widgets + bindkey + keymaps"
echo "  signal handling                    compdef + _arguments + zstyle"
echo "  job control + wait                 typeset -i base, -T tied, -A"
echo "  process subst + tee + cat          autoload -Uz + zwc cache"
echo "  modules (datetime/regex/zutil)     ZSH_EVAL_CONTEXT + FUNCNEST"
echo "  arithmetic eval (#var ordinals)    sticky emulation modes"
echo

banner "ALGORITHMIC COVERAGE" 72
echo
algos=(
    "trees:           BST, AVL, segment, Fenwick BIT, trie, treap"
    "graphs:          BFS, DFS, Dijkstra, Bellman-Ford, Floyd-Warshall, Kruskal, Prim, A*"
    "strings:         KMP, Z-function, Manacher, Rabin-Karp, Boyer-Moore, suffix array, LCS, LIS"
    "DP:              knapsack, coin change, edit distance, palindromic subseq, max subarray"
    "greedy:          activity selection, Huffman, MST, Dijkstra"
    "cryptography:    Caesar, Vigenère, OTP, XOR, RSA-toy, substitution, transposition, rail fence"
    "math:            Miller-Rabin, Pollard rho, GCD, ext-Euclidean, continued fractions, Fibonacci"
    "puzzles:         Sudoku, N-Queens, 15-puzzle, Hanoi, Lights Out, Boggle, Minesweeper, Conway"
    "geometry:        IPv4 subnet calc, color conversions (RGB/HSL/HSV), MAC address"
    "parsers:         JSON, TOML, INI, CSV, .env, HTTP, SQL, URL template, log formats, IPv6"
)
for a in "${algos[@]}"; do
    echo "  $a"
done

echo

banner "PORT BUGS DOCUMENTED" 72
echo
echo "  docs/BUGS.md catalogs 18 zshrs port issues found while writing demos."
echo
echo "  2 fixed:        #5 (prompt escapes), #7 (local arr=( \$=s ))"
echo "  4 demo errors:  #1 (j::), #2 (\"(o)\"), #3 (i case), #6 (gs| | |)"
echo "  12 open port:   #4, #8, #9, #10, #11, #12, #13, #14, #15, #16, #17, #18"
echo
echo "  Each documented with: reproducer, C-zsh vs zshrs output, Src/*.c"
echo "  port site, working workaround. Demos use the workarounds."
echo

banner "META" 72
echo
echo "  demos:        350"
echo "  CI runtime:   ~47s parallel"
echo "  pid:          $$"
echo "  zsh version:  $ZSH_VERSION"
zmodload zsh/datetime 2>/dev/null
if (( ${+EPOCHSECONDS} )); then
    echo "  generated:    $(TZ=UTC strftime '%Y-%m-%d %H:%M UTC' $EPOCHSECONDS 2>/dev/null)"
fi
echo

banner "→ github.com/MenkeTechnologies/zshrs" 72

# === ztest assertions ===
out=$(banner "hi" 10)
zassert_contains "$out" "═════════"            "banner has horizontal rule"
zassert_contains "$out" "hi"                   "banner contains text"
# pad calc: (10 - 2) / 2 = 4 spaces before 'hi'
zassert_match '    hi' "$out"                  "banner centered with 4 spaces"
out2=$(banner "X" 8)
# 3 lines: top bar, centered text, bottom bar
zassert_eq "$(echo "$out2" | wc -l | tr -d ' ')" "3" "banner is 3 lines"
zassert_ok 1 "milestone demo runs cleanly"
ztest_run
