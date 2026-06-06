#!/usr/bin/env zshrs
# Grand finale v5 — 310 demos.

banner() {
    local txt=$1 width=${2:-70}
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

   ██████╗ ███╗   ██╗███████╗███████╗
   ╚════██╗████╗  ██║██╔════╝██╔════╝
    █████╔╝██╔██╗ ██║█████╗  █████╗
    ╚═══██╗██║╚██╗██║██╔══╝  ██╔══╝
   ██████╔╝██║ ╚████║██║     ██║
   ╚═════╝ ╚═╝  ╚═══╝╚═╝     ╚═╝

EOF

banner "🚀 zshrs DEMO HARNESS: 310 STRONG 🚀" 70
echo
echo "  zshrs is the first compiled Unix shell — a drop-in Rust port"
echo "  of zsh with bytecode + fusevm JIT + AOP intercepts + worker"
echo "  pool. This demo suite pins zshrs's compat surface against"
echo "  zsh's upstream Src/*.c source, exercising every feature in"
echo "  isolation. Every demo runs on CI via examples_demos_ci.rs."
echo

banner "BATCH 12 (286-310) THEMES" 70
batches=(
    "286|segment tree (range sum + point update)"
    "287|Fenwick BIT tree + inversion count"
    "288|KMP string matching + failure table"
    "289|Rabin-Karp rolling-hash search"
    "290|Manacher palindrome O(n)"
    "291|reservoir sampling (Algorithm R)"
    "292|probabilistic skip list"
    "293|suffix array + LCP via Kasai"
    "294|word ladder via BFS"
    "295|Soundex phonetic hash"
    "296|Minesweeper flood reveal"
    "297|Mastermind w/ black/white peg scoring"
    "298|Tic-tac-toe minimax (perfect play)"
    "299|Conway's Life animated multi-pattern"
    "300|🎉 milestone-300 banner"
    "301|URL template (RFC 6570 subset)"
    "302|mini SQL SELECT parser + executor"
    "303|print -P / -v / -aC / -z flags"
    "304|unalias/unset/unhash/unfunction -m pattern"
    "305|typeset -m / -i / -F / -T / -A"
    "306|zle widget def + bindkey + keymaps"
    "307|compdef + _arguments + zstyle contexts"
    "308|graph density vs MST cost"
    "309|finite state machine DSL"
    "310|grand finale v5 banner"
)
for b in "${batches[@]}"; do
    printf "  %s  %s\n" "${b%%|*}" "${b#*|}"
done

echo
banner "CUMULATIVE BREAKDOWN" 70
prev=(
    "001-030|shell fundamentals"
    "031-060|algorithms + data structures"
    "061-085|zsh C feature pins (modifiers, glob, flags)"
    "086-110|advanced runtime (subshells, FDs, traps)"
    "111-135|extension + utility (zparseopts, modules)"
    "136-160|systems + apps (signals, processes)"
    "161-185|utilities + meta-programming"
    "186-210|parsers + apps (xxd, URL, JSON, CSV)"
    "211-235|apps + games (chess, tic-tac-toe, quiz)"
    "236-260|hooks + cryptography + grids + parsers"
    "261-285|crypto + graphs + games + zsh hooks"
    "286-310|trees + games + strings + zsh internals"
)
printf "  %-10s | %s\n" "Range" "Theme"
printf "  %-10s + %s\n" "──────────" "─────"
for b in "${prev[@]}"; do
    printf "  %-10s | %s\n" "${b%%|*}" "${b#*|}"
done

echo
banner "PORT BUGS SURFACED + DOCUMENTED" 70
echo
echo "  docs/BUGS.md catalogs 12 issues found while writing demos:"
echo
echo "  Demo-errors (zsh agrees, demo was wrong): #1 #2 #3 #6"
echo "  Port-bugs (zshrs diverges from zsh):"
echo "    #4  anon-fn in cmd-sub silent abort"
echo "    #5  print -P %j/%T/%D{} not handled  ✓ FIXED"
echo "    #7  local arr=( \$=s ) skips word-split  ✓ FIXED"
echo "    #8  local IFS= leaks past return"
echo "    #9  \${arr[(expr)*N+M]} unquoted → empty"
echo "    #10 nested-for + cmd-sub in fn corrupts iter"
echo "    #11 printf \"%d\" \"' \" returns 0 not 32"
echo "    #12 \${var%|*} treats | as glob alt"
echo
echo "  Each entry: reproducer, C-zsh expected vs zshrs observed,"
echo "  Src/*.c port site, working workaround."
echo

banner "META" 70
echo
echo "  demos:        310"
echo "  pid:          $$"
echo "  zsh version:  $ZSH_VERSION"
zmodload zsh/datetime 2>/dev/null
if (( ${+EPOCHSECONDS} )); then
    echo "  generated:    $(TZ=UTC strftime '%Y-%m-%d %H:%M UTC' $EPOCHSECONDS 2>/dev/null)"
fi
echo "  CI runtime:   ~30s (310 demos + coverage pin)"
echo

banner "→ github.com/MenkeTechnologies/zshrs" 70

# === ztest assertions ===
zassert_eq "${#batches}" "25"            "25 demos in batch 12 (286-310)"
zassert_eq "${#prev}"    "12"            "12 prior ranges listed"
out=$(banner "hi" 10)
zassert_contains "$out" "hi"             "banner contains text"
zassert_eq "$(echo "$out" | wc -l | tr -d ' ')" "3"  "banner 3 lines"
# Check first batch entry parses cleanly
first="${batches[1]}"
zassert_eq "${first%%|*}" "286"          "first batch entry = 286"
ztest_run
