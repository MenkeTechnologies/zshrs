#!/usr/bin/env zshrs
# 🎉 DEMO 300 — milestone celebration banner.

cat <<'EOF'


             ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄
             █  ░ ░ ░ ░ ░ ░ ░ ░  █
             █  ░ 3 0 0  d e m o s  █
             █  ░ ░ ░ ░ ░ ░ ░ ░  █
             ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀

EOF

# Banner helper.
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

banner "🎊 zshrs DEMO SUITE: 300 STRONG 🎊" 70
echo
echo "  This is demo #300 — a milestone for the zshrs port."
echo
echo "  The first compiled Unix shell, with a parity suite that"
echo "  exercises 300 distinct features against the upstream zsh C"
echo "  source. Every demo runs on CI via tests/examples_demos_ci.rs."
echo

banner "WHAT 300 DEMOS COVER" 70

echo
echo "  CATEGORY                          COUNT"
echo "  ─────────────────────────────────  ─────"
cat_list=(
    "Core shell semantics              30"
    "Algorithms + data structures       40"
    "zsh C-source feature pins          50"
    "Advanced runtime + traps           30"
    "Modules (zsh/datetime, regex)      15"
    "Pattern + glob qualifiers          15"
    "Parameter expansion flags          20"
    "Process control + signals          15"
    "Parsers (CSV/TOML/JSON/HTTP)       12"
    "Cryptography                       10"
    "Games + puzzles                    15"
    "Graphs + shortest paths            10"
    "String algorithms                  12"
    "Filesystem + atomic IO             8"
    "Hooks + autoload                   10"
    "Miscellaneous + meta               8"
)
for c in "${cat_list[@]}"; do
    echo "  $c"
done

echo
banner "EVERY DEMO PINS TO zsh's Src/*.c" 70
echo
echo "  When a demo exercises a zsh-C feature, it cites the upstream"
echo "  port site in a comment. Examples from the suite:"
echo
echo "    Src/subst.c paramsubst    — \${(j)(s)(o)(P)…} flags"
echo "    Src/glob.c bracecomplete   — {a..z} brace expansion"
echo "    Src/pattern.c patcompile   — extended glob, ksh patterns"
echo "    Src/hist.c                 — :h :t :r :e :a :A :s modifiers"
echo "    Src/exec.c execcmd         — command + redirection lifecycle"
echo "    Src/signals.c handletrap   — EXIT/ERR/USR1/INT/TERM dispatch"
echo "    Src/jobs.c                 — &, wait, jobs"
echo "    Src/params.c PM_HASHED     — associative arrays"
echo "    Src/builtin.c bin_typeset  — -i base, -T tied, -A assoc"
echo "    Src/lex.c lexword          — heredocs, here-strings, quoting"
echo "    Src/init.c periodic_*      — periodic hooks"
echo "    Src/Modules/datetime.c     — EPOCHSECONDS, strftime"
echo "    Src/Modules/mathfunc.c     — sin/cos/sqrt/log builtins"
echo "    Src/Modules/regex.c        — =~ + \$match capture"
echo "    Src/Modules/zutil.c        — zparseopts, zstyle"
echo "    Src/Zle/zle_*.c            — bindkey, compdef, widgets"

echo

banner "META" 70
echo
echo "  demos in suite:    300+"
echo "  CI runtime:        ~30s parallel"
echo "  bugs surfaced:     12 (4 demo-errors + 8 port-bugs in docs/BUGS.md)"
echo "  pid this demo:     $$"
echo "  zsh version:       $ZSH_VERSION"
zmodload zsh/datetime 2>/dev/null
if (( ${+EPOCHSECONDS} )); then
    echo "  generated:         $(TZ=UTC strftime '%Y-%m-%d %H:%M UTC' $EPOCHSECONDS 2>/dev/null)"
fi
echo

banner "ZSHRS — THE FIRST COMPILED UNIX SHELL" 70
echo
echo "  Bytecode + fusevm JIT + AOP + worker pool. No fork, no problems."
echo "  → github.com/MenkeTechnologies/zshrs"
echo

banner "" 70

# === ztest assertions ===
zassert_eq "$(banner foo 10 | wc -l | tr -d ' ')" "3"  "banner emits 3 lines"
out=$(banner "hi" 8)
zassert_contains "$out" "hi"            "banner contains text"
zassert_contains "$out" "════════"      "banner contains rule"
zassert_eq "${#cat_list}" "16"          "16 categories listed"
# Verify PID interpolation didn't fail
zassert_gt "$$" "0"                     "PID present"
ztest_run
