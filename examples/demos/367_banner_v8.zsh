#!/usr/bin/env zshrs
# Grand finale v8 — 367 demos.

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

  ████████╗ ███████╗  ███████╗
  ╚══██╔══╝██╔════╝ ████╔════╝
     ██║   ███████╗ █████████╗
     ██║   ╚════██║██╔════████║
     ██║   ███████║╚████████╔╝
     ╚═╝   ╚══════╝ ╚══════╝

EOF

banner "🎯 zshrs DEMO HARNESS: 367 STRONG 🎯" 72
echo
echo "  zshrs — first compiled Unix shell. Rust port of zsh."
echo "  This 367-demo suite pins zshrs against zsh's C source."
echo "  Every demo runs on CI; every demo is real functional code."
echo

banner "BATCH 15 (361-367) — SUBSTANTIAL FUNCTIONAL DEMOS" 72
echo
echo "  This batch focuses on FULL implementations (500+ lines each)"
echo "  with real tokenizers, parsers, evaluators, solvers:"
echo
batches=(
    "361|JSON parser (RFC 7159) — tokenizer + AST + JSONPath + pretty-print"
    "362|XML parser — tags + attrs + CDATA + comments + entity decode + XPath"
    "363|arithmetic expression evaluator — Shunting-yard RPN + functions + vars"
    "364|CSV parser (RFC 4180) — state machine + quoted fields + CRLF"
    "365|mini-Lisp interpreter — closures + recursion + cond/let/lambda/define"
    "366|Sudoku solver — backtracking + row/col/block validation"
    "367|grand finale 367-demo banner v8"
)
for b in "${batches[@]}"; do
    printf "  %s  %s\n" "${b%%|*}" "${b#*|}"
done

echo
banner "CUMULATIVE BREAKDOWN" 72
prev=(
    "001-030|shell fundamentals"
    "031-060|algorithms + data structures"
    "061-085|zsh C feature pins"
    "086-110|advanced runtime"
    "111-135|extension + utility"
    "136-160|systems + apps"
    "161-185|utilities + meta"
    "186-210|parsers + apps"
    "211-235|apps + games + introspection"
    "236-260|hooks + cryptography + grids + parsers"
    "261-285|crypto + graphs + games + zsh hooks"
    "286-310|trees + games + strings + zsh internals"
    "311-335|trees + DP + zsh deep dives"
    "336-360|math + parsers + ciphers + zsh metadata"
    "361-367|substantial parsers + evaluators (500+ LOC each)"
)
printf "  %-10s | %s\n" "Range" "Theme"
printf "  %-10s + %s\n" "──────────" "─────"
for b in "${prev[@]}"; do
    printf "  %-10s | %s\n" "${b%%|*}" "${b#*|}"
done

echo
banner "PORT BUGS — 20 DOCUMENTED IN docs/BUGS.md" 72
echo
echo "  Fixed (2):"
echo "    #5  print -P %j/%T/%D{} not handled"
echo "    #7  local arr=( \$=s ) skips word-split"
echo
echo "  Open port-bugs (14):"
echo "    #4  \$(() {…} arg) anon-fn cmd-sub silent abort"
echo "    #8  local IFS=: leaks past return"
echo "    #9  \${arr[(expr)*N+M]} unquoted returns empty"
echo "    #10 nested-for + cmd-sub in fn corrupts iter"
echo "    #11 printf \"%d\" \"' \" returns 0 instead of 32"
echo "    #12 \${var%|*} treats | as glob alt"
echo "    #13 [[ \"\$x\" == \"?\" ]] ignores quotes"
echo "    #14 [[ \$ch == \"{\" ]] parse error"
echo "    #15 set -- \${=x} mis-iterates in fn"
echo "    #16 arr=(\"\${arr[@]:0:-1}\") no-shrink in fn"
echo "    #17 var=\${arr[-1]} unquoted in fn while loops"
echo "    #18 arr[a + 1]=val with spaces parsed as cmd"
echo "    #19 case pattern \"!\" / \"if\" triggers parse error"
echo "    #20 recursive parsers run very slowly vs C-zsh"
echo
echo "  Demo errors (4): #1, #2, #3, #6 — zsh-correct behavior"
echo

banner "META" 72
echo
echo "  demos:        367"
echo "  pid:          $$"
echo "  zsh version:  $ZSH_VERSION"
zmodload zsh/datetime 2>/dev/null
if (( ${+EPOCHSECONDS} )); then
    echo "  generated:    $(TZ=UTC strftime '%Y-%m-%d %H:%M UTC' $EPOCHSECONDS 2>/dev/null)"
fi
echo "  bugs found:   20 documented in docs/BUGS.md"
echo

banner "→ github.com/MenkeTechnologies/zshrs" 72

# === ztest assertions ===
out=$(banner "x" 8)
zassert_contains "$out" "═══════"            "banner has horizontal rule"
zassert_contains "$out" "x"                  "banner contains text"
zassert_eq "$(echo "$out" | wc -l | tr -d ' ')" "3" "banner 3 lines"
zassert_eq "${batches[1]%%|*}" "361"          "batches[1] = 361"
zassert_eq "${batches[7]%%|*}" "367"          "batches[7] = 367"
zassert_eq "${#batches}" "7"                  "7 batch entries (361-367)"
zassert_eq "${prev[15]%%|*}" "361-367"        "prev[15] = 361-367 range"
ztest_run
