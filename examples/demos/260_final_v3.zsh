#!/usr/bin/env zshrs
# Grand finale v3 — 260 demos pinning zshrs to zsh.

ascii_logo() {
    cat <<'EOF'

     ██████  ███████ ██   ██ ██████  ███████
        ███  ██      ██   ██ ██   ██ ██
       ███   ███████ ███████ ██████  ███████
      ███         ██ ██   ██ ██   ██      ██
     ██████  ███████ ██   ██ ██   ██ ███████

EOF
}

banner() {
    local txt=$1 width=${2:-66}
    local n=${#txt}
    local pad=$(( (width - n) / 2 ))
    local sp=""; local bar=""; local i
    for ((i=0; i<pad; i++)); do sp+=" "; done
    for ((i=0; i<width; i++)); do bar+="═"; done
    echo "$bar"
    echo "${sp}${txt}"
    echo "$bar"
}

ascii_logo

banner "🚀 260 DEMOS — ZSHRS PARITY HARNESS 🚀" 66
echo
echo "  All demos run on CI via tests/examples_demos_ci.rs"
echo "  Every demo cites the Src/*.c source it exercises."
echo "  Coverage pin asserts on-disk ≡ registered."
echo

banner "BATCH BREAKDOWN" 66
batches=(
    "001-030|fundamentals (arrays, params, control flow)"
    "031-060|algorithms + data structures"
    "061-085|zsh C features (modifiers, glob qualifiers, flags)"
    "086-110|advanced runtime (subshells, FDs, traps, jobs)"
    "111-135|extension + utility (zparseopts, modules, autoload)"
    "136-160|systems + apps (signals, processes, networking)"
    "161-185|utilities + meta-programming"
    "186-210|parsers + apps + meta"
    "211-235|meta + games + apps"
    "236-260|hooks + cryptography + grids + parsers"
)
printf "  %-10s | %s\n" "Range" "Theme"
printf "  %-10s + %s\n" "──────────" "─────"
for b in "${batches[@]}"; do
    set -- ${(s:|:)b}
    printf "  %-10s | %s\n" "$1" "$2"
done

echo
banner "COVERAGE HIGHLIGHTS" 66
features=(
    "arithmetic: int, float, bases, math funcs (Src/math.c)"
    "arrays: indexed, assoc, slices, parens, $=, flags"
    "patterns: glob, extended, ksh, qualifiers, modifiers"
    "param expansion: 30+ flags (Src/subst.c paramsubst)"
    "control flow: if/case/for/while/until/repeat/select"
    "redirection: <, >, >>, &>, 2>&1, <(), >()"
    "background: &, wait, jobs, kill, fg/bg"
    "signals: USR1/2, HUP, TERM, INT, EXIT, ERR, ZERR"
    "modules: datetime, mathfunc, regex, zutil, parameter"
    "builtins: typeset, let, read, print, printf, getopts"
    "options: setopt, local_options, emulate, kshoptarg"
    "hooks: precmd, preexec, chpwd via add-zsh-hook"
    "autoload: fpath search, -U/-z flags, lazy fns"
    "compsys: compdef, _arguments, _values, _files"
    "cryptography: Caesar, Vigenère, substitution, ROT13"
    "graphs: BFS, DFS, Dijkstra, MST, topological sort"
    "puzzles: Sudoku, Lights Out, Boggle, Hangman"
    "parsers: TOML, INI, .env, JSON, CSV, XML, IPv6"
)
for f in "${features[@]}"; do
    echo "  ✓ $f"
done

echo
banner "STATS" 66
echo
echo "  demos:       260"
echo "  pid:         $$"
echo "  zsh version: $ZSH_VERSION"
echo "  argv0:       $0"
echo "  PWD:         ${PWD/#$HOME/~}"
echo

zmodload zsh/datetime 2>/dev/null
if (( ${+EPOCHSECONDS} )); then
    echo "  generated:   $(TZ=UTC strftime '%Y-%m-%d %H:%M UTC' $EPOCHSECONDS 2>/dev/null)"
fi
echo "  CI runtime:  ~30s (256 demos + coverage pin)"

echo
banner "ZSHRS: THE FIRST COMPILED UNIX SHELL" 66
echo
echo "  Drop-in zsh replacement in Rust."
echo "  Bytecode + fusevm + AOP + worker pool."
echo "  No fork, no problems."
echo
echo "  → github.com/MenkeTechnologies/zshrs"
echo

banner "" 66

# === ztest assertions ===
zassert_eq "${#batches[@]}"  "10" "10 batches listed"
zassert_eq "${#features[@]}" "18" "18 coverage highlights"
zassert_ok "${functions[ascii_logo]:+1}" "ascii_logo defined"
zassert_ok "${functions[banner]:+1}"     "banner defined"
out=$(banner hello 20)
zassert_contains "$out" "hello" "banner contains the text"
zassert_contains "$out" "═"     "banner draws double-line border"
zassert_contains "${batches[1]}" "001-030" "first batch covers 001-030"
zassert_contains "${batches[10]}" "236-260" "last batch covers 236-260"
zassert_contains "${features[1]}" "arithmetic" "feature 1 mentions arithmetic"
zassert_ok "$$" "PID is set"
ztest_run
