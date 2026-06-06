#!/usr/bin/env zshrs
# Final recap — celebrates 185 demos of zshrs coverage.

zmodload zsh/datetime 2>/dev/null

batches=(
    "01-30:fundamentals"
    "31-60:algorithms+structures"
    "61-85:zsh-C-features"
    "86-110:advanced-runtime"
    "111-135:extension-patterns"
    "136-160:systems+apps"
    "161-185:utilities+meta"
)

echo "── zshrs demo suite recap ──"
echo
printf "%-12s %s\n" "Batch" "Theme"
printf "%-12s %s\n" "─────" "─────"
for b in "${batches[@]}"; do
    printf "%-12s %s\n" "${b%%:*}" "${b#*:}"
done

echo
echo "── total: 185 demos across 7 batches ──"
echo "── all run on CI via tests/examples_demos_ci.rs ──"

echo
echo "── language features exercised ──"
features=(
    "arithmetic: integer + float + bases (Src/math.c)"
    "arrays: 1-based + negative indexing + slicing (Src/params.c)"
    "associative arrays: PM_HASHED + (@kv) (Src/params.c)"
    "parameter expansion: 30+ flags (Src/subst.c paramsubst)"
    "globbing: extended + qualifiers + recursive (Src/glob.c)"
    "patterns: ^ ~ # ## (a|b) ?(p) *(p) +(p) !(p) (Src/pattern.c)"
    "modifiers: :h :t :r :e :a :A :s (Src/hist.c)"
    "control flow: if/case/for/while/until/repeat (Src/parse.c)"
    "redirection: < > >> &> 2>&1 (Src/exec.c addfd)"
    "subshell: () vs {} (Src/exec.c execsubsh)"
    "command-sub: \$(…) (Src/exec.c getoutput)"
    "process-sub: <(…) >(…) (Src/exec.c getoutputfile)"
    "background+wait: &/wait (Src/jobs.c)"
    "traps: EXIT/ERR/ZERR/USR1 (Src/signals.c handletrap)"
    "modules: zsh/datetime, zsh/mathfunc, zsh/regex, zsh/zutil"
    "builtins: typeset, let, read, print, printf, getopts, exec"
    "options: setopt/local_options/emulate -L (Src/options.c)"
)
for f in "${features[@]}"; do
    echo "  ✓ $f"
done

echo
echo "── meta ──"
echo "  generated: $(TZ=UTC strftime '%Y-%m-%d %H:%M UTC' $EPOCHSECONDS 2>/dev/null)"
echo "  pid:       $$"
echo "  argv0:     $0"
echo "  zshrs:     $ZSH_VERSION"

echo
echo "── thank you ──"
echo "  ───────────────────────────"
echo "  zshrs: the FIRST compiled"
echo "  Unix shell — drop-in zsh"
echo "  replacement in Rust."
echo "  ───────────────────────────"

# === ztest assertions ===
zassert_eq "${#batches[@]}" 7        "7 batches recorded"
zassert_eq "${batches[1]%%:*}" "01-30"   "first batch range"
zassert_eq "${batches[1]#*:}"  "fundamentals" "first batch theme"
zassert_eq "${batches[7]%%:*}" "161-185" "last batch range"
zassert_ok "${#features[@]}"        "features array populated"
zassert_ge "${#features[@]}" 15     "at least 15 features listed"
zassert_contains "${features[1]}" "arithmetic" "first feature mentions arithmetic"
ztest_run
