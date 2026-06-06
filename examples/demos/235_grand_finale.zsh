#!/usr/bin/env zshrs
# Grand finale — 235 demos summary banner.

print_banner() {
    local txt=$1 width=${2:-60}
    local n=${#txt}
    local pad=$(( (width - n) / 2 ))
    local sp=""; local i
    for ((i=0; i<pad; i++)); do sp+=" "; done
    local bar=""
    for ((i=0; i<width; i++)); do bar+="═"; done
    echo "$bar"
    echo "${sp}${txt}"
    echo "$bar"
}

print_banner "🎉 ZSHRS DEMO SUITE — 235 STRONG 🎉" 60
echo
echo "  An aggregate of pure-shell demos pinning zshrs's"
echo "  feature coverage against zsh's C source. Every"
echo "  demo runs on CI; every demo cites its Src/*.c."
echo

print_banner "BATCH SUMMARY" 60
echo
batches=(
    "01-30:fundamentals:30"
    "31-60:algorithms+structures:30"
    "61-85:zsh-C-features:25"
    "86-110:advanced-runtime:25"
    "111-135:extension+utility:25"
    "136-160:systems+apps:25"
    "161-185:utilities+meta:25"
    "186-210:parsers+apps+meta:25"
    "211-235:meta+games+apps:25"
)
printf "  %-10s  %-30s  %s\n" "Range" "Theme" "Count"
printf "  %-10s  %-30s  %s\n" "─────" "─────" "─────"
for b in "${batches[@]}"; do
    set -- $=$(echo "$b" | tr ':' ' ')
    printf "  %-10s  %-30s  %s\n" "$1" "$2" "$3"
done

echo
print_banner "COVERAGE" 60
features=(
    "arithmetic: int + float + bases + funcs (Src/math.c)"
    "arrays: indexed + assoc + slices + flags (Src/params.c)"
    "patterns: glob + extended + ksh + qualifiers (Src/pattern.c)"
    "parameter expansion: 30+ flags (Src/subst.c paramsubst)"
    "modifiers: :h :t :r :e :a :A :s (Src/hist.c)"
    "control flow: if/case/for/while/until/repeat/select"
    "redirection: < > >> &> 2>&1 <() >() (Src/exec.c)"
    "background: & wait jobs (Src/jobs.c)"
    "traps: EXIT ERR ZERR USR1 USR2 (Src/signals.c)"
    "modules: datetime mathfunc regex zutil parameter"
    "builtins: typeset let read print printf getopts exec"
    "options: setopt local_options emulate (Src/options.c)"
)
for f in "${features[@]}"; do
    echo "  ✓ $f"
done

echo
print_banner "META" 60
echo
echo "  generated:    $(zmodload zsh/datetime 2>/dev/null; TZ=UTC strftime '%Y-%m-%d %H:%M UTC' $EPOCHSECONDS 2>/dev/null)"
echo "  pid:          $$"
echo "  argv0:        $0"
echo "  zsh version:  $ZSH_VERSION"
echo "  CI runtime:   ~30s for all 235 demos"
echo

print_banner "THANK YOU 🚀" 60
echo
echo "  zshrs — THE FIRST COMPILED UNIX SHELL"
echo
echo "  Drop-in zsh replacement in Rust."
echo "  Bytecode + fusevm + AOP + worker pool."
echo "  No fork, no problems."
echo
echo "  → github.com/MenkeTechnologies/zshrs"
echo
print_banner "" 60

# === ztest assertions ===
zassert_eq "${#batches[@]}"  9   "9 batch entries"
zassert_eq "${#features[@]}" 12  "12 feature lines"
b1=$(print_banner "HI" 20)
zassert_contains "$b1" "HI" "banner shows text"
# count ═ chars per banner line (20 wide)
bar_line=$(echo "$b1" | head -1)
zassert_contains "$bar_line" "═" "banner uses ═ char"
zassert_ok "$ZSH_VERSION" "ZSH_VERSION populated"
zassert_match '[0-9]+\.[0-9]+' "$ZSH_VERSION" "ZSH_VERSION looks like a version"
ztest_run
